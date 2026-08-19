use std::path::Path;

use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const DEFAULT_ALPHABET: &str = "asdfwerzxcuioptbm";
const LEGACY_DEFAULT_ALPHABET: &str = "asdfwerzxvjkluopghtyb";
const RESERVED_HINT_KEYS: &str = "ghjklnqvy";

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
    pub popup: PopupSize,
    pub profiles: Vec<PopupProfile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PopupSize {
    pub width: String,
    pub height: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PopupProfile {
    pub name: String,
    pub min_client_width: Option<u16>,
    pub max_client_width: Option<u16>,
    pub width: String,
    pub height: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            alphabet: DEFAULT_ALPHABET.chars().collect(),
            patterns: definitions(BUILTIN_PATTERNS).collect(),
            popup: PopupSize {
                width: "90%".into(),
                height: "90%".into(),
            },
            profiles: vec![
                PopupProfile {
                    name: "laptop".into(),
                    min_client_width: None,
                    max_client_width: Some(310),
                    width: "95%".into(),
                    height: "90%".into(),
                },
                PopupProfile {
                    name: "partial-ultrawide".into(),
                    min_client_width: None,
                    max_client_width: Some(350),
                    width: "90%".into(),
                    height: "90%".into(),
                },
                PopupProfile {
                    name: "full-ultrawide".into(),
                    min_client_width: Some(400),
                    max_client_width: None,
                    width: "70%".into(),
                    height: "90%".into(),
                },
            ],
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
        let defaults = Self::default();
        let configured_alphabet = file
            .alphabet
            .unwrap_or_else(|| defaults.alphabet.iter().collect());
        let alphabet = if configured_alphabet == LEGACY_DEFAULT_ALPHABET {
            DEFAULT_ALPHABET
        } else {
            &configured_alphabet
        }
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

        let popup = file.popup.unwrap_or(defaults.popup);
        let profiles = file.profiles.unwrap_or(defaults.profiles);
        validate_popup(&popup)?;
        validate_profiles(&profiles)?;

        Ok(Self {
            alphabet,
            patterns,
            popup,
            profiles,
        })
    }

    pub fn popup(&self, client_width: Option<u16>) -> PopupSize {
        let Some(width) = client_width else {
            return self.popup.clone();
        };
        self.profiles
            .iter()
            .find(|profile| profile.matches(width))
            .map(PopupProfile::size)
            .unwrap_or_else(|| self.popup.clone())
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    alphabet: Option<String>,
    enabled_builtin_patterns: Option<Vec<String>>,
    #[serde(default)]
    patterns: Vec<PatternDefinition>,
    popup: Option<PopupSize>,
    profiles: Option<Vec<PopupProfile>>,
}

impl PopupProfile {
    fn matches(&self, width: u16) -> bool {
        self.min_client_width.is_none_or(|min| width >= min)
            && self.max_client_width.is_none_or(|max| width <= max)
    }

    fn size(&self) -> PopupSize {
        PopupSize {
            width: self.width.clone(),
            height: self.height.clone(),
        }
    }
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
    if alphabet.iter().any(|key| !key.is_ascii_lowercase()) {
        bail!("alphabet must use lower-case ASCII keys");
    }
    let conflicts = alphabet
        .iter()
        .copied()
        .filter(|key| RESERVED_HINT_KEYS.contains(*key))
        .collect::<String>();
    if !conflicts.is_empty() {
        bail!("alphabet contains reserved normal-mode keys: {conflicts}");
    }
    Ok(())
}

fn validate_profiles(profiles: &[PopupProfile]) -> Result<()> {
    for profile in profiles {
        if profile.name.trim().is_empty() {
            bail!("popup profile name cannot be empty");
        }
        if profile
            .min_client_width
            .zip(profile.max_client_width)
            .is_some_and(|(min, max)| min > max)
        {
            bail!(
                "popup profile '{}' has min width above max width",
                profile.name
            );
        }
        validate_popup(&profile.size())?;
    }
    Ok(())
}

fn validate_popup(popup: &PopupSize) -> Result<()> {
    for (name, value) in [("width", &popup.width), ("height", &popup.height)] {
        let valid = value.strip_suffix('%').map_or_else(
            || value.parse::<u16>().is_ok_and(|cells| cells > 0),
            |percent| {
                percent
                    .parse::<u16>()
                    .is_ok_and(|number| (1..=100).contains(&number))
            },
        );
        if !valid {
            bail!("popup {name} '{value}' must be positive cells or 1%-100%");
        }
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

    #[test]
    fn popup_profiles_are_responsive_and_first_match_wins() {
        let config = Config::default();

        assert_eq!(config.popup(Some(300)).width, "95%");
        assert_eq!(config.popup(Some(330)).width, "90%");
        assert_eq!(config.popup(Some(380)).width, "90%");
        assert_eq!(config.popup(Some(512)).width, "70%");
        assert_eq!(config.popup(Some(512)).height, "90%");
    }

    #[test]
    fn hint_alphabet_rejects_normal_mode_commands() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "alphabet = \"asj\"\n").unwrap();

        let error = Config::load(&path).unwrap_err().to_string();

        assert!(error.contains("reserved"));
        assert!(error.contains('j'));
    }

    #[test]
    fn legacy_default_alphabet_migrates_to_the_safe_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "alphabet = \"asdfwerzxvjkluopghtyb\"\n").unwrap();

        let config = Config::load(&path).unwrap();

        assert_eq!(
            config.alphabet,
            DEFAULT_ALPHABET.chars().collect::<Vec<_>>()
        );
    }
}
