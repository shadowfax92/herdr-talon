//! Copy through the host clipboard utility. Linux utilities keep a background
//! selection owner alive after the popup exits, so copied text remains available.
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

use crate::app::Completion;

#[derive(Clone, Debug)]
struct ClipboardCommand {
    program: PathBuf,
    args: Vec<String>,
}

/// Tries utilities for the current display server without invoking a shell.
/// Copy failures remain visible in the picker and do not discard its selection.
#[derive(Clone, Debug)]
pub struct ActionExecutor {
    commands: Vec<ClipboardCommand>,
}

impl ActionExecutor {
    pub fn new() -> Self {
        if cfg!(target_os = "macos") {
            Self::with_command("pbcopy")
        } else {
            Self::linux(
                std::env::var_os("WAYLAND_DISPLAY").is_some_and(|s| !s.is_empty())
                    || std::env::var_os("WAYLAND_SOCKET").is_some_and(|s| !s.is_empty()),
                std::env::var_os("DISPLAY").is_some_and(|s| !s.is_empty()),
            )
        }
    }

    fn linux(wayland: bool, x11: bool) -> Self {
        let mut commands = Vec::new();
        if wayland {
            commands.push(ClipboardCommand {
                program: "wl-copy".into(),
                args: vec!["--type".into(), "text/plain;charset=utf-8".into()],
            });
        }
        if x11 {
            for (program, args) in [
                ("xclip", ["-selection", "clipboard"]),
                ("xsel", ["--clipboard", "--input"]),
            ] {
                commands.push(ClipboardCommand {
                    program: program.into(),
                    args: args.into_iter().map(str::to_owned).collect(),
                });
            }
        }
        Self { commands }
    }

    pub fn with_command(program: impl Into<PathBuf>) -> Self {
        Self {
            commands: vec![ClipboardCommand {
                program: program.into(),
                args: Vec::new(),
            }],
        }
    }

    pub fn execute(&self, completion: &Completion) -> Result<()> {
        if self.commands.is_empty() {
            bail!("No graphical clipboard is available: set WAYLAND_DISPLAY or DISPLAY for your Linux/WSLg session");
        }
        let mut errors = Vec::new();
        for command in &self.commands {
            match command.copy(&completion.text) {
                Ok(()) => return Ok(()),
                Err(error) => errors.push(format!("{}: {error:#}", command.program.display())),
            }
        }
        bail!("Clipboard copy failed: {}", errors.join("; "))
    }
}

impl ClipboardCommand {
    fn copy(&self, text: &str) -> Result<()> {
        // A clipboard owner's daemon may inherit stderr after its parent exits.
        // A private temporary file lets us collect failures without waiting for
        // pipe EOF from that long-lived owner, which would hang the popup.
        let mut diagnostics = tempfile::tempfile().context("creating clipboard diagnostics")?;
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(diagnostics.try_clone()?)
            .spawn()
            .context("starting clipboard utility")?;
        let write_result = child
            .stdin
            .take()
            .context("clipboard stdin is unavailable")?
            .write_all(text.as_bytes());
        let status = child.wait().context("waiting for clipboard utility")?;
        if !status.success() {
            diagnostics.seek(SeekFrom::Start(0))?;
            let mut error = String::new();
            diagnostics.take(16 * 1024).read_to_string(&mut error)?;
            bail!("exited with {status}: {}", error.trim());
        }
        write_result.context("writing clipboard text")
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
            text: "two words\n日本語 λ\n".into(),
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

    #[test]
    fn linux_falls_back_without_changing_clipboard_text_or_arguments() {
        let directory = tempdir().unwrap();
        let mut executor = ActionExecutor::linux(true, true);
        assert_eq!(
            executor
                .commands
                .iter()
                .map(|c| c.program.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["wl-copy", "xclip", "xsel"]
        );
        for command in &mut executor.commands {
            command.program = directory.path().join(&command.program);
        }
        // wl-copy is missing and xclip cannot connect. xsel must still get the
        // same bytes and target CLIPBOARD rather than the primary selection.
        executable(&executor.commands[1].program, "#!/bin/sh\nexit 9\n");
        executable(
            &executor.commands[2].program,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$0.args\"\ncat > \"$0.out\"\n",
        );
        let text = "literal $(never-run) `not-code` 日本語\n\n";
        executor.execute(&Completion { text: text.into() }).unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("xsel.out")).unwrap(),
            text
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("xsel.args")).unwrap(),
            "--clipboard\n--input\n"
        );
    }

    #[test]
    fn clipboard_daemon_cannot_hold_the_popup_open_through_stderr() {
        let directory = tempdir().unwrap();
        let command = directory.path().join("clipboard");
        executable(
            &command,
            "#!/bin/sh\ncat > \"$0.out\"\nsleep 30 &\necho $! > \"$0.pid\"\n",
        );
        let executor = ActionExecutor::with_command(&command);
        let started = std::time::Instant::now();
        let result = executor.execute(&Completion {
            text: "keep".into(),
        });
        let elapsed = started.elapsed();
        let pid = std::fs::read_to_string(directory.path().join("clipboard.pid")).unwrap();
        // Only the child PID written by our own fixture is terminated.
        Command::new("kill").arg(pid.trim()).status().unwrap();
        result.unwrap();
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "clipboard owner kept diagnostics open for {elapsed:?}"
        );
    }

    #[test]
    fn linux_without_a_display_reports_an_actionable_error() {
        let error = ActionExecutor::linux(false, false)
            .execute(&Completion {
                text: "keep".into(),
            })
            .unwrap_err();
        assert!(error.to_string().contains("WAYLAND_DISPLAY or DISPLAY"));
    }
}
