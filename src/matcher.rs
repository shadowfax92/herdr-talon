use std::collections::HashMap;

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthStr;

use crate::config::PatternDefinition;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Occurrence {
    pub row: usize,
    pub highlight_col: usize,
    pub highlight_width: usize,
    pub hint_col: usize,
    pub hint_width: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Target {
    pub text: String,
    pub occurrences: Vec<Occurrence>,
}

pub fn find_targets(text: &str, patterns: &[PatternDefinition]) -> Result<Vec<Target>> {
    let patterns = patterns
        .iter()
        .map(|pattern| {
            Regex::new(&pattern.regex)
                .with_context(|| format!("invalid regex for pattern '{}'", pattern.name))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut targets = Vec::<Target>::new();
    let mut target_by_text = HashMap::<String, usize>::new();

    for (row, line) in text.split_terminator('\n').enumerate() {
        let mut candidates = Vec::new();
        for (priority, pattern) in patterns.iter().enumerate() {
            for captures in pattern.captures_iter(line) {
                let Some(full) = captures.get(0) else {
                    continue;
                };
                let captured = captures.name("match").unwrap_or(full);
                if captured.as_str().is_empty() {
                    continue;
                }
                candidates.push(Candidate {
                    priority,
                    full_start: full.start(),
                    full_end: full.end(),
                    capture_start: captured.start(),
                    capture_end: captured.end(),
                    text: captured.as_str().to_string(),
                });
            }
        }
        candidates.sort_by_key(|candidate| {
            (
                candidate.full_start,
                candidate.priority,
                std::cmp::Reverse(candidate.full_end),
            )
        });

        let mut occupied_until = 0;
        for candidate in candidates {
            if candidate.full_start < occupied_until {
                continue;
            }
            occupied_until = candidate.full_end;
            let occurrence = Occurrence {
                row,
                highlight_col: cell_width(&line[..candidate.full_start]),
                highlight_width: cell_width(&line[candidate.full_start..candidate.full_end]).max(1),
                hint_col: cell_width(&line[..candidate.capture_start]),
                hint_width: cell_width(&line[candidate.capture_start..candidate.capture_end])
                    .max(1),
            };
            if let Some(index) = target_by_text.get(&candidate.text).copied() {
                targets[index].occurrences.push(occurrence);
            } else {
                let index = targets.len();
                target_by_text.insert(candidate.text.clone(), index);
                targets.push(Target {
                    text: candidate.text,
                    occurrences: vec![occurrence],
                });
            }
        }
    }

    Ok(targets)
}

struct Candidate {
    priority: usize,
    full_start: usize,
    full_end: usize,
    capture_start: usize,
    capture_end: usize,
    text: String,
}

fn cell_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use tempfile::tempdir;

    use super::*;

    fn pattern(name: &str, regex: &str) -> PatternDefinition {
        PatternDefinition {
            name: name.into(),
            regex: regex.into(),
        }
    }

    #[test]
    fn named_capture_controls_copied_text_and_hint_position() {
        let targets = find_targets(
            "modified: src/main.rs",
            &[pattern("status", r"modified: (?<match>.+)")],
        )
        .unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].text, "src/main.rs");
        assert_eq!(targets[0].occurrences[0].highlight_col, 0);
        assert_eq!(targets[0].occurrences[0].highlight_width, 21);
        assert_eq!(targets[0].occurrences[0].hint_col, 10);
        assert_eq!(targets[0].occurrences[0].hint_width, 11);
    }

    #[test]
    fn earlier_patterns_win_same_start_and_leftmost_matches_win_overlap() {
        let targets = find_targets(
            "abc123 xyz",
            &[
                pattern("specific", r"abc(?<match>[0-9]+)"),
                pattern("whole", r"abc123"),
                pattern("overlap", r"123 xyz"),
            ],
        )
        .unwrap();

        assert_eq!(
            targets
                .iter()
                .map(|target| target.text.as_str())
                .collect::<Vec<_>>(),
            vec!["123"]
        );
    }

    #[test]
    fn repeated_text_shares_one_target_with_all_occurrences() {
        let targets =
            find_targets("deadbeef x\ndeadbeef y", &[pattern("sha", r"[0-9a-f]{8}")]).unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].text, "deadbeef");
        assert_eq!(targets[0].occurrences.len(), 2);
        assert_eq!(targets[0].occurrences[1].row, 1);
    }

    #[test]
    fn unicode_byte_offsets_become_terminal_cell_offsets() {
        let targets = find_targets("λ 日本語 /tmp/a", &[pattern("path", r"/[^ ]+")]).unwrap();

        assert_eq!(targets[0].occurrences[0].hint_col, 9);
        assert_eq!(targets[0].occurrences[0].hint_width, 6);
    }

    #[test]
    fn source_coordinates_do_not_saturate_on_long_logical_lines() {
        let text = format!("{} /tmp/a", "x".repeat(70_000));
        let targets = find_targets(&text, &[pattern("path", r"/[^ ]+")]).unwrap();

        assert_eq!(targets[0].occurrences[0].hint_col as usize, 70_001);
    }

    #[test]
    fn empty_captures_are_ignored() {
        let targets = find_targets("abc", &[pattern("empty", r"(?<match>)abc")]).unwrap();

        assert!(targets.is_empty());
    }

    #[test]
    fn approved_defaults_match_representative_values() {
        let config = Config::default();
        let cases = [
            ("ip", "10.0.0.1", "10.0.0.1"),
            ("digit", "ticket 1234", "1234"),
            ("url", "https://herdr.dev/docs/", "https://herdr.dev/docs/"),
            ("path", "./src/main.rs", "./src/main.rs"),
            ("hex", "0xDEAD", "0xDEAD"),
            ("kubernetes", "deployment.apps/api", "deployment.apps/api"),
            ("git-status", "modified: src/lib.rs", "src/lib.rs"),
            (
                "git-status-branch",
                "Your branch is up to date with 'main'.",
                "main",
            ),
            ("diff", "+++ b/src/main.rs", "src/main.rs"),
        ];

        for (name, input, expected) in cases {
            let definition = config
                .patterns
                .iter()
                .find(|pattern| pattern.name == name)
                .unwrap();
            let targets = find_targets(input, std::slice::from_ref(definition)).unwrap();
            assert_eq!(targets[0].text, expected, "pattern {name}");
        }
    }

    #[test]
    fn example_config_provides_fingers_compatible_patterns() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, include_str!("../talon.toml.example")).unwrap();
        let config = Config::load(&path).unwrap();
        let cases = [
            (
                "git-ssh",
                "git@example.test:owner/repo.git",
                "git@example.test:owner/repo.git",
            ),
            (
                "user-uuid",
                "123e4567-e89b-12d3-a456-426614174000",
                "123e4567-e89b-12d3-a456-426614174000",
            ),
            ("user-sha", "deadbeef", "deadbeef"),
            ("file-line", "/tmp/main.rs:42", "/tmp/main.rs:42"),
        ];

        for (name, input, expected) in cases {
            let definition = config
                .patterns
                .iter()
                .find(|pattern| pattern.name == name)
                .unwrap();
            let targets = find_targets(input, std::slice::from_ref(definition)).unwrap();
            assert_eq!(targets[0].text, expected, "pattern {name}");
        }

        let targets = find_targets(
            "Your VMs:\n  • atlas-node.example.test - running (example/runtime)\n  • comet-node.example.test - running (example/runtime)",
            &config.patterns,
        )
        .unwrap();
        let values = targets
            .iter()
            .map(|target| target.text.as_str())
            .collect::<Vec<_>>();

        assert!(values.contains(&"atlas-node.example.test"));
        assert!(values.contains(&"comet-node.example.test"));
    }
}
