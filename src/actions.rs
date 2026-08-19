use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::app::Completion;

#[derive(Clone, Debug)]
pub struct ActionExecutor {
    pbcopy: PathBuf,
}

impl ActionExecutor {
    pub fn new() -> Self {
        Self::with_command("pbcopy")
    }

    pub fn with_command(pbcopy: impl Into<PathBuf>) -> Self {
        Self {
            pbcopy: pbcopy.into(),
        }
    }

    pub fn execute(&self, completion: &Completion) -> Result<()> {
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
            .write_all(completion.text.as_bytes())?;
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
}

impl Default for ActionExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;

    fn executable(path: &Path, source: &str) {
        std::fs::write(path, source).unwrap();
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[test]
    fn copies_exact_multiline_text_to_the_clipboard() {
        let directory = tempdir().unwrap();
        let pbcopy = directory.path().join("pbcopy");
        executable(&pbcopy, "#!/bin/sh\ncat > \"$0.out\"\n");
        let executor = ActionExecutor::with_command(&pbcopy);
        let completion = Completion {
            text: "two words\nnext line".into(),
        };

        executor.execute(&completion).unwrap();

        assert_eq!(
            std::fs::read_to_string(directory.path().join("pbcopy.out")).unwrap(),
            completion.text
        );
    }

    #[test]
    fn reports_clipboard_failure() {
        let directory = tempdir().unwrap();
        let pbcopy = directory.path().join("pbcopy");
        executable(
            &pbcopy,
            "#!/bin/sh\nprintf 'clipboard unavailable' >&2\nexit 9\n",
        );
        let executor = ActionExecutor::with_command(&pbcopy);

        let error = executor
            .execute(&Completion {
                text: "keep me".into(),
            })
            .unwrap_err()
            .to_string();

        assert!(error.contains("clipboard unavailable"));
    }
}
