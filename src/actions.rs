use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::app::{ActionKind, Completion};
use crate::herdr::Herdr;

#[derive(Clone, Debug)]
pub struct ActionExecutor {
    herdr: Herdr,
    source_pane_id: String,
    source_cwd: Option<PathBuf>,
    pbcopy: PathBuf,
    open: PathBuf,
}

impl ActionExecutor {
    pub fn new(
        herdr: Herdr,
        source_pane_id: impl Into<String>,
        source_cwd: Option<PathBuf>,
    ) -> Self {
        Self::with_commands(herdr, source_pane_id, source_cwd, "pbcopy", "open")
    }

    pub fn with_commands(
        herdr: Herdr,
        source_pane_id: impl Into<String>,
        source_cwd: Option<PathBuf>,
        pbcopy: impl Into<PathBuf>,
        open: impl Into<PathBuf>,
    ) -> Self {
        Self {
            herdr,
            source_pane_id: source_pane_id.into(),
            source_cwd,
            pbcopy: pbcopy.into(),
            open: open.into(),
        }
    }

    pub fn execute(&self, completion: &Completion) -> Result<()> {
        self.copy(&completion.text)?;
        match completion.kind {
            ActionKind::Copy | ActionKind::MultiCopy => Ok(()),
            ActionKind::Paste => self.herdr.send_text(&self.source_pane_id, &completion.text),
            ActionKind::Open => self.open(&completion.text),
        }
    }

    fn copy(&self, text: &str) -> Result<()> {
        let mut child = Command::new(&self.pbcopy)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to start {}", self.pbcopy.display()))?;
        child
            .stdin
            .take()
            .context("pbcopy stdin is unavailable")?
            .write_all(text.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            bail!(
                "pbcopy failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }

    fn open(&self, text: &str) -> Result<()> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let target = resolve_open_target(text, self.source_cwd.as_deref(), home.as_deref());
        let output = Command::new(&self.open)
            .arg(&target)
            .output()
            .with_context(|| format!("failed to start {}", self.open.display()))?;
        if !output.status.success() {
            bail!(
                "open failed with {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

pub fn resolve_open_target(value: &str, cwd: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if let Some((path, line)) = value.rsplit_once(':') {
        if line.parse::<u64>().is_ok() {
            let target = resolve_path(path, cwd, home);
            if target.exists() {
                return target;
            }
        }
    }
    resolve_path(value, cwd, home)
}

fn resolve_path(value: &str, cwd: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if value == "~" {
        return home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = home {
            return home.join(rest);
        }
    }
    let value = PathBuf::from(value);
    if value.is_relative() {
        if let Some(candidate) = cwd.map(|cwd| cwd.join(&value)).filter(|path| path.exists()) {
            return candidate;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    use crate::app::ActionKind;

    use super::*;

    fn executable(path: &Path, source: &str) {
        std::fs::write(path, source).unwrap();
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    fn executor() -> (tempfile::TempDir, ActionExecutor) {
        let dir = tempdir().unwrap();
        let pbcopy = dir.path().join("pbcopy");
        let open = dir.path().join("open");
        let herdr = dir.path().join("herdr");
        executable(&pbcopy, "#!/bin/sh\ncat > \"$0.out\"\n");
        executable(&open, "#!/bin/sh\nprintf '%s' \"$1\" > \"$0.arg\"\n");
        executable(&herdr, "#!/bin/sh\nprintf '%s' \"$4\" > \"$0.text\"\n");
        let executor = ActionExecutor::with_commands(
            Herdr::new(&herdr),
            "w1:p1",
            Some(dir.path().to_path_buf()),
            &pbcopy,
            &open,
        );
        (dir, executor)
    }

    #[test]
    fn paste_copies_first_then_sends_text_without_enter() {
        let (dir, executor) = executor();
        let completion = Completion {
            kind: ActionKind::Paste,
            text: "two words\nnext".into(),
        };

        executor.execute(&completion).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("pbcopy.out")).unwrap(),
            completion.text
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("herdr.text")).unwrap(),
            completion.text
        );
    }

    #[test]
    fn open_copies_first_and_resolves_existing_relative_paths() {
        let (dir, executor) = executor();
        let relative = Path::new("src/main.rs");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join(relative), "fn main() {}").unwrap();

        executor
            .execute(&Completion {
                kind: ActionKind::Open,
                text: relative.display().to_string(),
            })
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("open.arg")).unwrap(),
            dir.path().join(relative).display().to_string()
        );
    }

    #[test]
    fn clipboard_failure_prevents_follow_up_actions() {
        let (dir, executor) = executor();
        executable(&dir.path().join("pbcopy"), "#!/bin/sh\nexit 9\n");

        assert!(executor
            .execute(&Completion {
                kind: ActionKind::Open,
                text: "https://herdr.dev".into(),
            })
            .is_err());
        assert!(!dir.path().join("open.arg").exists());
    }

    #[test]
    fn tilde_and_non_paths_resolve_safely() {
        let cwd = Path::new("/tmp/project");
        let home = Path::new("/Users/tester");

        assert_eq!(
            resolve_open_target("~/notes.md", Some(cwd), Some(home)),
            PathBuf::from("/Users/tester/notes.md")
        );
        assert_eq!(
            resolve_open_target("https://herdr.dev/docs", Some(cwd), Some(home)),
            PathBuf::from("https://herdr.dev/docs")
        );
    }

    #[test]
    fn existing_file_line_targets_open_the_file() {
        let directory = tempdir().unwrap();
        let file = directory.path().join("src/main.rs");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "fn main() {}").unwrap();

        assert_eq!(
            resolve_open_target("src/main.rs:42", Some(directory.path()), None),
            file
        );
        assert_eq!(
            resolve_open_target("https://localhost:8080", Some(directory.path()), None),
            PathBuf::from("https://localhost:8080")
        );
    }
}
