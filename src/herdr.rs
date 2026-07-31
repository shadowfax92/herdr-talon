use std::path::{Path, PathBuf};

use std::ffi::{OsStr, OsString};
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::PLUGIN_ID;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LayoutPane {
    pub pane_id: String,
    pub focused: bool,
    pub rect: Rect,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Layout {
    pub workspace_id: String,
    pub tab_id: String,
    pub zoomed: bool,
    pub area: Rect,
    pub focused_pane_id: String,
    pub panes: Vec<LayoutPane>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PaneInfo {
    pub pane_id: String,
    pub cwd: Option<String>,
    pub viewport_rows: u16,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct InvocationContext {
    pub focused_pane_id: Option<String>,
    pub focused_pane_cwd: Option<String>,
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
    pub fn new(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
        }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub fn layout(&self, pane_id: &str) -> Result<Layout> {
        let response: LayoutEnvelope = self.json(["pane", "layout", "--pane", pane_id])?;
        Ok(response.result.layout)
    }

    pub fn pane_info(&self, pane_id: &str) -> Result<PaneInfo> {
        let response: PaneEnvelope = self.json(["pane", "get", pane_id])?;
        let viewport_rows = response
            .result
            .pane
            .scroll
            .map(|scroll| u16::try_from(scroll.viewport_rows).unwrap_or(u16::MAX))
            .unwrap_or(0);
        Ok(PaneInfo {
            pane_id: response.result.pane.pane_id,
            cwd: response.result.pane.cwd,
            viewport_rows,
        })
    }

    pub fn read_visible(&self, pane_id: &str, ansi: bool) -> Result<String> {
        let format = if ansi { "ansi" } else { "text" };
        let output = self.output([
            OsStr::new("pane"),
            OsStr::new("read"),
            OsStr::new(pane_id),
            OsStr::new("--source"),
            OsStr::new("visible"),
            OsStr::new("--format"),
            OsStr::new(format),
        ])?;
        String::from_utf8(output).context("Herdr pane read returned non-UTF-8 output")
    }

    pub fn open_picker(&self, run_id: &str) -> Result<()> {
        let environment = format!("HERDR_TALON_RUN_ID={run_id}");
        self.output([
            OsStr::new("plugin"),
            OsStr::new("pane"),
            OsStr::new("open"),
            OsStr::new("--plugin"),
            OsStr::new(PLUGIN_ID),
            OsStr::new("--entrypoint"),
            OsStr::new("picker"),
            OsStr::new("--placement"),
            OsStr::new("overlay"),
            OsStr::new("--env"),
            OsStr::new(&environment),
        ])?;
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

    pub fn send_text(&self, pane_id: &str, text: &str) -> Result<()> {
        self.output([
            OsStr::new("pane"),
            OsStr::new("send-text"),
            OsStr::new(pane_id),
            OsStr::new(text),
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
        let args = args
            .into_iter()
            .map(|value| value.as_ref().to_os_string())
            .collect::<Vec<OsString>>();
        let output = Command::new(&self.binary)
            .args(&args)
            .output()
            .with_context(|| format!("failed to run {}", self.binary.display()))?;
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
}

#[derive(Deserialize)]
struct LayoutEnvelope {
    result: LayoutResult,
}

#[derive(Deserialize)]
struct LayoutResult {
    layout: Layout,
}

#[derive(Deserialize)]
struct PaneEnvelope {
    result: PaneResult,
}

#[derive(Deserialize)]
struct PaneResult {
    pane: RawPaneInfo,
}

#[derive(Deserialize)]
struct RawPaneInfo {
    pane_id: String,
    cwd: Option<String>,
    scroll: Option<RawScroll>,
}

#[derive(Deserialize)]
struct RawScroll {
    viewport_rows: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_context_requires_a_nonempty_focused_pane() {
        let context = InvocationContext::parse(
            r#"{"focused_pane_id":"w2:p1","focused_pane_cwd":"/tmp/project"}"#,
        )
        .unwrap();

        assert_eq!(context.source_pane_id().unwrap(), "w2:p1");
        assert_eq!(context.focused_pane_cwd.as_deref(), Some("/tmp/project"));
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
}
