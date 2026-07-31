use std::path::Path;

use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const DEFAULT_ALPHABET: &str = "asdfwerzxvjkluopghtyb";

const USER_PATTERNS: &[(&str, &str)] = &[
    ("git-ssh", r"git@[a-zA-Z0-9.-]+:[a-zA-Z0-9/.-]+\.git"),
    (
        "user-uuid",
        r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
    ),
    ("user-sha", r"[0-9a-f]{7,40}"),
    ("file-line", r"/[^ \t\r\n]+:[0-9]+"),
];

const BUILTIN_PATTERNS: &[(&str, &str)] = &[
    ("ip", r"\d{1,3}(?:\.\d{1,3}){3}"),
    (
        "uuid",
        r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
    ),
    ("sha", r"[0-9a-f]{7,128}"),
    ("digit", r"[0-9]{4,}"),
    (
        "url",
        r#"(?:https?://|git@|git://|ssh://|ftp://|file:///)[^\s()"']+"#,
    ),
    ("path", r"(?:[.\w\-~$@]+)?(?:/[.\w\-@]+)+/?"),
    ("hex", r"0x[0-9a-fA-F]+"),
    (
        "kubernetes",
        r"(?:binding|componentstatuses|configmap|endpoints?|events?|limitrange|namespaces?|nodes?|persistentvolumeclaims?|persistentvolumes?|pods?|podtemplates?|replicationcontrollers?|resourcequotas?|secrets?|serviceaccounts?|services?|daemonset\.apps|deployment\.apps|replicaset\.apps|statefulset\.apps|cronjob\.batch|job\.batch|ingress\.networking\.k8s\.io|networkpolicy\.networking\.k8s\.io|clusterrolebindings?\.rbac\.authorization\.k8s\.io|roles?\.rbac\.authorization\.k8s\.io|customresourcedefinitions?\.apiextensions\.k8s\.io)[[:alnum:]_#$%&+=/@.-]+",
    ),
    (
        "git-status",
        r"(?:modified|deleted|deleted by us|new file): +(?<match>.+)",
    ),
    (
        "git-status-branch",
        r"Your branch is up to date with '(?<match>.*)'\.",
    ),
    ("diff", r"(?:---|\+\+\+) [ab]/(?<match>.*)"),
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PatternDefinition {
    pub name: String,
    pub regex: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub alphabet: Vec<char>,
    pub patterns: Vec<PatternDefinition>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            alphabet: DEFAULT_ALPHABET.chars().collect(),
            patterns: definitions(USER_PATTERNS)
                .chain(definitions(BUILTIN_PATTERNS))
                .collect(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let source = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let file: FileConfig = toml::from_str(&source)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let alphabet = file
            .alphabet
            .unwrap_or_else(|| DEFAULT_ALPHABET.to_string())
            .chars()
            .collect::<Vec<_>>();
        validate_alphabet(&alphabet)?;

        let enabled = file.enabled_builtin_patterns.unwrap_or_else(|| {
            BUILTIN_PATTERNS
                .iter()
                .map(|(name, _)| (*name).to_string())
                .collect()
        });
        let mut patterns = file.patterns;
        patterns.extend(definitions(USER_PATTERNS));
        for name in enabled {
            let Some((_, source)) = BUILTIN_PATTERNS
                .iter()
                .find(|(candidate, _)| *candidate == name)
            else {
                bail!("unknown built-in pattern '{name}'");
            };
            patterns.push(PatternDefinition {
                name,
                regex: (*source).to_string(),
            });
        }
        validate_patterns(&patterns)?;

        Ok(Self { alphabet, patterns })
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    alphabet: Option<String>,
    enabled_builtin_patterns: Option<Vec<String>>,
    #[serde(default)]
    patterns: Vec<PatternDefinition>,
}

fn definitions(
    patterns: &'static [(&'static str, &'static str)],
) -> impl Iterator<Item = PatternDefinition> {
    patterns.iter().map(|(name, regex)| PatternDefinition {
        name: (*name).to_string(),
        regex: (*regex).to_string(),
    })
}

fn validate_alphabet(alphabet: &[char]) -> Result<()> {
    if alphabet.len() < 2 {
        bail!("alphabet must contain at least two keys");
    }
    let unique = alphabet.iter().copied().collect::<HashSet<_>>();
    if unique.len() != alphabet.len() {
        bail!("alphabet keys must be unique");
    }
    if alphabet
        .iter()
        .any(|key| !key.is_ascii_lowercase() || *key == 'q')
    {
        bail!("alphabet must use lower-case ASCII keys and cannot contain q");
    }
    Ok(())
}

fn validate_patterns(patterns: &[PatternDefinition]) -> Result<()> {
    let mut names = HashSet::new();
    for pattern in patterns {
        if pattern.name.trim().is_empty() {
            bail!("pattern names cannot be empty");
        }
        if !names.insert(pattern.name.as_str()) {
            bail!("duplicate pattern name '{}'", pattern.name);
        }
        regex::Regex::new(&pattern.regex)
            .with_context(|| format!("invalid regex for pattern '{}'", pattern.name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn defaults_include_every_approved_pattern() {
        let config = Config::default();
        let names = config
            .patterns
            .iter()
            .map(|pattern| pattern.name.as_str())
            .collect::<HashSet<_>>();

        for expected in [
            "git-ssh",
            "user-uuid",
            "user-sha",
            "file-line",
            "ip",
            "uuid",
            "sha",
            "digit",
            "url",
            "path",
            "hex",
            "kubernetes",
            "git-status",
            "git-status-branch",
            "diff",
        ] {
            assert!(names.contains(expected), "missing {expected}");
        }
        assert!(config.alphabet.len() >= 10);
        assert!(!config.alphabet.contains(&'q'));
    }

    #[test]
    fn custom_patterns_are_ordered_before_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
alphabet = "abc"
enabled_builtin_patterns = ["url"]

[[patterns]]
name = "ticket"
regex = "TKT-[0-9]+"
"#,
        )
        .unwrap();

        let config = Config::load(&path).unwrap();

        assert_eq!(config.alphabet, vec!['a', 'b', 'c']);
        assert_eq!(config.patterns[0].name, "ticket");
        assert!(config.patterns.iter().any(|pattern| pattern.name == "url"));
        assert!(!config.patterns.iter().any(|pattern| pattern.name == "ip"));
    }

    #[test]
    fn invalid_regex_and_alphabet_fail_during_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
alphabet = "aa"

[[patterns]]
name = "broken"
regex = "("
"#,
        )
        .unwrap();

        let error = Config::load(&path).unwrap_err().to_string();

        assert!(error.contains("alphabet") || error.contains("broken"));
    }
}
