use std::path::PathBuf;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event};

use crate::{
    actions::ActionExecutor,
    app::{Completion, InputOutcome, PickerState},
    herdr::Herdr,
    snapshot::{RunSnapshot, RunStore},
    ui::{viewport_size, TalonView},
};

pub fn run_from_environment() -> Result<()> {
    let herdr = Herdr::from_environment();
    let result = (|| {
        let state_dir = std::env::var_os("HERDR_PLUGIN_STATE_DIR")
            .map(PathBuf::from)
            .context("HERDR_PLUGIN_STATE_DIR is not set")?;
        let run_id =
            std::env::var("HERDR_TALON_RUN_ID").context("HERDR_TALON_RUN_ID is not set")?;
        let store = RunStore::new(state_dir)?;
        let picker_result = (|| {
            let snapshot = store.claim(&run_id)?;
            let executor = ActionExecutor::new();
            ratatui::run(|terminal| run_picker(terminal, &snapshot, &executor))
        })();
        let cleanup_result = store.clear_active_picker(&run_id);
        picker_result?;
        cleanup_result
    })();
    if let Err(error) = &result {
        let _ = herdr.notify(&format!("Talon picker failed: {error:#}"));
    }
    result
}

fn run_picker(
    terminal: &mut ratatui::DefaultTerminal,
    snapshot: &RunSnapshot,
    executor: &ActionExecutor,
) -> Result<()> {
    let mut state = PickerState::new(snapshot)?;
    loop {
        terminal.draw(|frame| {
            let (width, height) = viewport_size(frame.area());
            state.set_viewport(width, height);
            frame.render_widget(TalonView::new(snapshot, &state), frame.area());
        })?;
        let event = event::read()?;
        if matches!(event, Event::Resize(_, _)) {
            terminal.autoresize()?;
            continue;
        }
        match state.handle_event(event) {
            InputOutcome::Continue => {}
            InputOutcome::Cancel => return Ok(()),
            InputOutcome::Complete(completion) => {
                if finish_copy(&mut state, executor, &completion) {
                    return Ok(());
                }
            }
        }
    }
}

fn finish_copy(
    state: &mut PickerState,
    executor: &ActionExecutor,
    completion: &Completion,
) -> bool {
    match executor.execute(completion) {
        Ok(()) => true,
        Err(error) => {
            state.set_error(format!("Copy failed: {error:#}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn clipboard_failure_keeps_the_picker_open_with_an_error() {
        let directory = tempdir().unwrap();
        let pbcopy = directory.path().join("pbcopy");
        std::fs::write(&pbcopy, "#!/bin/sh\nexit 9\n").unwrap();
        let mut permissions = std::fs::metadata(&pbcopy).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&pbcopy, permissions).unwrap();
        let executor = ActionExecutor::with_command(pbcopy);
        let snapshot = RunSnapshot {
            source_pane_id: "w1:p1".into(),
            text: "value".into(),
            ansi: "value".into(),
            history_limited: false,
            targets: Vec::new(),
            alphabet: vec!['a', 's'],
        };
        let mut state = PickerState::new(&snapshot).unwrap();

        assert!(!finish_copy(
            &mut state,
            &executor,
            &Completion {
                text: "value".into()
            }
        ));
        assert!(state.error().unwrap().contains("Copy failed"));
    }
}
