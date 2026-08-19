use std::fs::{self, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::Config;
use crate::herdr::{ClosePluginPaneOutcome, Herdr, InvocationContext};
use crate::matcher::{find_targets, Target};

pub const CAPTURE_ROWS: u32 = 1_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RunSnapshot {
    pub source_pane_id: String,
    pub text: String,
    pub ansi: String,
    pub history_limited: bool,
    pub targets: Vec<Target>,
    pub alphabet: Vec<char>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchOutcome {
    Opened { run_id: String },
    Closed { pane_id: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct ActivePicker {
    run_id: String,
    pane_id: String,
}

#[derive(Clone, Debug)]
pub struct RunStore {
    root: PathBuf,
}

impl RunStore {
    pub fn new(state_dir: impl AsRef<Path>) -> Result<Self> {
        let state_dir = state_dir.as_ref();
        fs::create_dir_all(state_dir)
            .with_context(|| format!("failed to create {}", state_dir.display()))?;
        let root = state_dir.join("runs");
        match fs::symlink_metadata(&root) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    bail!("run directory is not a private directory");
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&root)
                    .with_context(|| format!("failed to create {}", root.display()))?;
            }
            Err(error) => return Err(error).context("failed to inspect run directory"),
        }
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn write(&self, snapshot: &RunSnapshot) -> Result<String> {
        self.reap_stale(Duration::from_secs(3600))?;
        let run_id = Uuid::new_v4().to_string();
        let final_path = self.path(&run_id)?;
        let temporary_path = self.root.join(format!(".{run_id}.tmp"));
        let result: Result<String> = (|| {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary_path)?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer(&mut writer, snapshot)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            if final_path.exists() {
                bail!("run ID collision");
            }
            fs::rename(&temporary_path, &final_path)?;
            Ok(run_id)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result.context("failed to write Talon handoff")
    }

    pub fn claim(&self, run_id: &str) -> Result<RunSnapshot> {
        let path = self.path(run_id)?;
        let metadata = fs::symlink_metadata(&path).context("Talon handoff does not exist")?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("Talon handoff is not a regular file");
        }
        let claimed = self
            .root
            .join(format!(".{run_id}.{}.claim", std::process::id()));
        fs::rename(&path, &claimed).context("Talon handoff was already claimed")?;
        let result = (|| {
            let metadata = fs::symlink_metadata(&claimed)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!("claimed Talon handoff is not a regular file");
            }
            let reader = BufReader::new(fs::File::open(&claimed)?);
            serde_json::from_reader(reader).context("failed to decode Talon handoff")
        })();
        let _ = fs::remove_file(&claimed);
        result
    }

    pub fn remove(&self, run_id: &str) -> Result<()> {
        let path = self.path(run_id)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("failed to remove Talon handoff"),
        }
    }

    pub fn set_active_picker(&self, run_id: &str, pane_id: &str) -> Result<()> {
        self.path(run_id)?;
        if pane_id.is_empty() || pane_id.chars().any(char::is_control) {
            bail!("invalid Talon picker pane ID");
        }
        let active = ActivePicker {
            run_id: run_id.to_string(),
            pane_id: pane_id.to_string(),
        };
        let final_path = self.active_picker_path();
        let temporary_path = self
            .root
            .join(format!(".active-picker.{}.tmp", std::process::id()));
        let result: Result<()> = (|| {
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary_path)?;
            let mut writer = BufWriter::new(file);
            serde_json::to_writer(&mut writer, &active)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            fs::rename(&temporary_path, &final_path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result.context("failed to record active Talon picker")
    }

    pub fn clear_active_picker(&self, run_id: &str) -> Result<()> {
        let Some(active) = self.active_picker()? else {
            return Ok(());
        };
        if active.run_id != run_id {
            return Ok(());
        }
        match fs::remove_file(self.active_picker_path()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).context("failed to clear active Talon picker"),
        }
    }

    pub fn reap_stale(&self, age: Duration) -> Result<usize> {
        let now = SystemTime::now();
        let mut removed = 0;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if entry.file_name() == ".active-picker.json" {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            let stale = metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|elapsed| elapsed >= age);
            if stale {
                fs::remove_file(entry.path())?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn path(&self, run_id: &str) -> Result<PathBuf> {
        let parsed = Uuid::parse_str(run_id).context("invalid Talon run ID")?;
        if parsed.to_string() != run_id {
            bail!("Talon run ID is not canonical");
        }
        Ok(self.root.join(format!("{run_id}.json")))
    }

    fn active_picker(&self) -> Result<Option<ActivePicker>> {
        let path = self.active_picker_path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to inspect active Talon picker"),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("active Talon picker record is not a regular file");
        }
        let active: ActivePicker = serde_json::from_reader(BufReader::new(fs::File::open(path)?))
            .context("failed to decode active Talon picker")?;
        self.path(&active.run_id)?;
        if active.pane_id.is_empty() || active.pane_id.chars().any(char::is_control) {
            bail!("active Talon picker has an invalid pane ID");
        }
        Ok(Some(active))
    }

    fn active_picker_path(&self) -> PathBuf {
        self.root.join(".active-picker.json")
    }
}

pub fn launch(
    herdr: &Herdr,
    context: &InvocationContext,
    config: &Config,
    store: &RunStore,
) -> Result<LaunchOutcome> {
    if let Some(active) = store.active_picker()? {
        match herdr.close_plugin_pane(&active.pane_id)? {
            ClosePluginPaneOutcome::Closed => {
                store.clear_active_picker(&active.run_id)?;
                return Ok(LaunchOutcome::Closed {
                    pane_id: active.pane_id,
                });
            }
            ClosePluginPaneOutcome::NotFound => {
                store.clear_active_picker(&active.run_id)?;
            }
        }
    }
    let source_pane_id = context.source_pane_id()?;
    let client_width = herdr.client_width(source_pane_id)?;
    let (text, ansi, history_limited) = capture_history(herdr, source_pane_id)?;
    let targets = find_targets(&text, &config.patterns)?;
    let snapshot = RunSnapshot {
        source_pane_id: source_pane_id.to_string(),
        text,
        ansi,
        history_limited,
        targets,
        alphabet: config.alphabet.clone(),
    };
    let run_id = store.write(&snapshot)?;
    let popup = config.popup(Some(client_width));
    let pane_id = match herdr.open_picker(&run_id, &popup) {
        Ok(pane_id) => pane_id,
        Err(error) => {
            let _ = store.remove(&run_id);
            return Err(error);
        }
    };
    if let Err(error) = store.set_active_picker(&run_id, &pane_id) {
        let _ = herdr.close_plugin_pane(&pane_id);
        let _ = store.remove(&run_id);
        return Err(error);
    }
    Ok(LaunchOutcome::Opened { run_id })
}

fn capture_history(herdr: &Herdr, pane_id: &str) -> Result<(String, String, bool)> {
    let recent_text =
        normalize_newlines(herdr.read_recent_unwrapped(pane_id, false, CAPTURE_ROWS)?);
    let recent_ansi =
        normalize_newlines(herdr.read_recent_unwrapped(pane_id, true, CAPTURE_ROWS)?);
    let (text, ansi) = if recent_text.trim().is_empty() {
        (
            normalize_newlines(herdr.read_visible(pane_id, false)?),
            normalize_newlines(herdr.read_visible(pane_id, true)?),
        )
    } else {
        (recent_text, recent_ansi)
    };
    let ansi = if row_count(&ansi) == row_count(&text) {
        ansi
    } else {
        text.clone()
    };
    let history_limited = row_count(&text) >= CAPTURE_ROWS as usize;
    Ok((text, ansi, history_limited))
}

fn normalize_newlines(text: String) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn row_count(text: &str) -> usize {
    text.split_terminator('\n').count().max(1)
}

pub fn launch_with_reporting(
    herdr: &Herdr,
    context_json: &str,
    config_path: &Path,
    state_dir: &Path,
) -> Result<LaunchOutcome> {
    let result = (|| {
        let context = InvocationContext::parse(context_json)?;
        let config = Config::load(config_path)?;
        let store = RunStore::new(state_dir)?;
        launch(herdr, &context, &config, &store)
    })();
    if let Err(error) = &result {
        let _ = herdr.notify(&format!("Talon could not start: {error:#}"));
    }
    result
}

pub fn launch_from_environment() -> Result<LaunchOutcome> {
    let herdr = Herdr::from_environment();
    let context = std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
        .context("HERDR_PLUGIN_CONTEXT_JSON is not set")?;
    let config_dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR")
        .map(PathBuf::from)
        .context("HERDR_PLUGIN_CONFIG_DIR is not set")?;
    let state_dir = std::env::var_os("HERDR_PLUGIN_STATE_DIR")
        .map(PathBuf::from)
        .context("HERDR_PLUGIN_STATE_DIR is not set")?;
    launch_with_reporting(
        &herdr,
        &context,
        &config_dir.join("config.toml"),
        &state_dir,
    )
}

#[cfg(test)]
mod tests {
    use std::fs::FileTimes;
    use std::os::unix::fs::MetadataExt;
    use std::time::SystemTime;

    use tempfile::tempdir;

    use crate::matcher::Occurrence;

    use super::*;

    fn sample_snapshot() -> RunSnapshot {
        RunSnapshot {
            source_pane_id: "w1:p1".into(),
            text: "deadbeef\n".into(),
            ansi: "\u{1b}[33mdeadbeef\u{1b}[0m\n".into(),
            history_limited: false,
            targets: vec![Target {
                text: "deadbeef".into(),
                occurrences: vec![Occurrence {
                    row: 0,
                    highlight_col: 0,
                    highlight_width: 8,
                    hint_col: 0,
                    hint_width: 8,
                }],
            }],
            alphabet: vec!['a', 's'],
        }
    }

    #[test]
    fn handoff_is_private_claimed_once_and_removed() {
        let dir = tempdir().unwrap();
        let store = RunStore::new(dir.path()).unwrap();
        let run_id = store.write(&sample_snapshot()).unwrap();
        let path = store.root().join(format!("{run_id}.json"));

        assert_eq!(
            std::fs::metadata(store.root()).unwrap().mode() & 0o777,
            0o700
        );
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(store.claim(&run_id).unwrap(), sample_snapshot());
        assert!(!path.exists());
        assert!(store.claim(&run_id).is_err());
    }

    #[test]
    fn invalid_ids_and_symlink_handoffs_are_rejected() {
        let dir = tempdir().unwrap();
        let store = RunStore::new(dir.path()).unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(&outside, serde_json::to_vec(&sample_snapshot()).unwrap()).unwrap();
        let run_id = uuid::Uuid::new_v4().to_string();
        std::os::unix::fs::symlink(&outside, store.root().join(format!("{run_id}.json"))).unwrap();

        assert!(store.claim("../../outside").is_err());
        assert!(store.claim(&run_id).is_err());
        assert!(outside.exists());
    }

    #[test]
    fn stale_regular_handoffs_are_reaped_without_following_links() {
        let dir = tempdir().unwrap();
        let store = RunStore::new(dir.path()).unwrap();
        let run_id = store.write(&sample_snapshot()).unwrap();
        let path = store.root().join(format!("{run_id}.json"));
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_times(
            FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(7200)),
        )
        .unwrap();
        let outside = dir.path().join("outside");
        std::fs::write(&outside, "keep").unwrap();
        let link = store.root().join("stale.json");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        assert_eq!(store.reap_stale(Duration::from_secs(3600)).unwrap(), 1);
        assert!(!path.exists());
        assert!(link.exists());
        assert!(outside.exists());
    }

    #[test]
    fn store_rejects_a_symlinked_run_directory() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("runs")).unwrap();

        assert!(RunStore::new(dir.path()).is_err());
    }

    #[test]
    fn active_picker_record_is_private_and_cleared_by_its_run() {
        let dir = tempdir().unwrap();
        let store = RunStore::new(dir.path()).unwrap();
        let run_id = Uuid::new_v4().to_string();
        let other_run_id = Uuid::new_v4().to_string();
        let path = store.active_picker_path();

        store.set_active_picker(&run_id, "w1:p3").unwrap();

        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        store.clear_active_picker(&other_run_id).unwrap();
        assert!(path.exists());
        store.clear_active_picker(&run_id).unwrap();
        assert!(!path.exists());
    }
}
