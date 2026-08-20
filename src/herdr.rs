use std::path::{Path, PathBuf};

use std::ffi::{OsStr, OsString};
use std::process::{Command, Output};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::config::PopupSize;
use crate::PLUGIN_ID;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct InvocationContext {
    pub focused_pane_id: Option<String>,
}

impl InvocationContext {
    pub fn parse(json: &str) -> Result<Self> {
        serde_json::from_str(json).context("invalid HERDR_PLUGIN_CONTEXT_JSON")
    }

    pub fn source_pane_id(&self) -> Result<&str> {
        let Some(pane_id) = self.focused_pane_id.as_deref().map(str::trim) else {
            bail!("plugin context has no focused pane");
        };
        if pane_id.is_empty() || pane_id.chars().any(char::is_control) {
            bail!("plugin context has no valid focused pane");
        }
        Ok(pane_id)
    }
}

#[derive(Clone, Debug)]
pub struct Herdr {
    binary: PathBuf,
}

impl Herdr {
    pub fn from_environment() -> Self {
        Self {
            binary: runtime_binary(std::env::var_os("HERDR_BIN_PATH")).into(),
        }
    }

    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub fn client_width(&self, pane_id: &str) -> Result<u16> {
        let response: LayoutEnvelope = self.json(["pane", "layout", "--pane", pane_id])?;
        Ok(response
            .result
            .layout
            .area
            .x
            .saturating_add(response.result.layout.area.width))
    }

    pub fn read_visible(&self, pane_id: &str, ansi: bool) -> Result<String> {
        self.read_pane(pane_id, "visible", ansi, None)
    }

    pub fn read_recent_unwrapped(&self, pane_id: &str, ansi: bool, lines: u32) -> Result<String> {
        self.read_pane(pane_id, "recent-unwrapped", ansi, Some(lines))
    }

    fn read_pane(
        &self,
        pane_id: &str,
        source: &str,
        ansi: bool,
        lines: Option<u32>,
    ) -> Result<String> {
        let format = if ansi { "ansi" } else { "text" };
        let mut args = vec![
            OsString::from("pane"),
            OsString::from("read"),
            OsString::from(pane_id),
            OsString::from("--source"),
            OsString::from(source),
        ];
        if let Some(lines) = lines {
            args.push(OsString::from("--lines"));
            args.push(OsString::from(lines.to_string()));
        }
        args.push(OsString::from("--format"));
        args.push(OsString::from(format));
        let output = self.output(args)?;
        String::from_utf8(output).context("Herdr pane read returned non-UTF-8 output")
    }

    pub fn open_picker(&self, run_id: &str, popup: &PopupSize) -> Result<()> {
        let environment = format!("HERDR_TALON_RUN_ID={run_id}");
        let response: OkEnvelope = self.json([
            OsStr::new("plugin"),
            OsStr::new("pane"),
            OsStr::new("open"),
            OsStr::new("--plugin"),
            OsStr::new(PLUGIN_ID),
            OsStr::new("--entrypoint"),
            OsStr::new("picker"),
            OsStr::new("--placement"),
            OsStr::new("popup"),
            OsStr::new("--width"),
            OsStr::new(&popup.width),
            OsStr::new("--height"),
            OsStr::new(&popup.height),
            OsStr::new("--env"),
            OsStr::new(&environment),
            OsStr::new("--focus"),
        ])?;
        if response.result.kind != "ok" {
            bail!("Herdr returned an unexpected popup response");
        }
        Ok(())
    }

    pub fn notify(&self, body: &str) -> Result<()> {
        let body = body.chars().take(220).collect::<String>();
        self.output([
            OsStr::new("notification"),
            OsStr::new("show"),
            OsStr::new("Talon"),
            OsStr::new("--body"),
            OsStr::new(&body),
        ])?;
        Ok(())
    }

    pub fn reload_config(&self) -> Result<()> {
        self.output([OsStr::new("server"), OsStr::new("reload-config")])?;
        Ok(())
    }

    fn json<T, I, S>(&self, args: I) -> Result<T>
    where
        T: for<'de> Deserialize<'de>,
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.output(args)?;
        serde_json::from_slice(&output).context("failed to decode Herdr JSON response")
    }

    fn output<I, S>(&self, args: I) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.command(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "Herdr command failed with {}: {}",
                output.status,
                stderr.trim()
            );
        }
        Ok(output.stdout)
    }

    fn command<I, S>(&self, args: I) -> Result<Output>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args = args
            .into_iter()
            .map(|value| value.as_ref().to_os_string())
            .collect::<Vec<OsString>>();
        Command::new(&self.binary)
            .args(&args)
            .output()
            .with_context(|| format!("failed to run {}", self.binary.display()))
    }
}

fn runtime_binary(injected: Option<OsString>) -> OsString {
    match injected {
        Some(binary) if !Path::new(&binary).is_absolute() || Path::new(&binary).is_file() => binary,
        _ => OsString::from("herdr"),
    }
}

#[derive(Deserialize)]
struct LayoutEnvelope {
    result: LayoutResult,
}

#[derive(Deserialize)]
struct LayoutResult {
    layout: ClientLayout,
}

#[derive(Deserialize)]
struct ClientLayout {
    area: ClientArea,
}

#[derive(Deserialize)]
struct ClientArea {
    x: u16,
    width: u16,
}

#[derive(Deserialize)]
struct OkEnvelope {
    result: OkResult,
}

#[derive(Deserialize)]
struct OkResult {
    #[serde(rename = "type")]
    kind: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn invocation_context_requires_a_nonempty_focused_pane() {
        let context = InvocationContext::parse(
            r#"{"focused_pane_id":"w2:p1","focused_pane_cwd":"/tmp/project"}"#,
        )
        .unwrap();

        assert_eq!(context.source_pane_id().unwrap(), "w2:p1");
        assert!(InvocationContext::parse("{}")
            .unwrap()
            .source_pane_id()
            .is_err());
        assert!(InvocationContext::parse(r#"{"focused_pane_id":"   "}"#)
            .unwrap()
            .source_pane_id()
            .is_err());
        assert!(InvocationContext::parse("not-json").is_err());
    }

    #[test]
    fn missing_injected_binary_falls_back_to_path_lookup() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("herdr-existing");
        std::fs::write(&existing, "").unwrap();

        assert_eq!(
            runtime_binary(Some(existing.clone().into_os_string())),
            existing.into_os_string()
        );
        assert_eq!(
            runtime_binary(Some(dir.path().join("herdr-missing").into_os_string())),
            OsString::from("herdr")
        );
        assert_eq!(
            runtime_binary(Some(OsString::from("herdr-custom"))),
            OsString::from("herdr-custom")
        );
    }
}
