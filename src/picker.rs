use std::path::PathBuf;

use anyhow::{Context, Result};
use ratatui::crossterm::event;

use crate::actions::ActionExecutor;
use crate::app::{InputOutcome, PickerState};
use crate::herdr::Herdr;
use crate::snapshot::{RunSnapshot, RunStore};
use crate::ui::TalonView;

pub fn run_from_environment() -> Result<()> {
    let binary = std::env::var_os("HERDR_BIN_PATH").unwrap_or_else(|| "herdr".into());
    let herdr = Herdr::new(binary);
    let result = (|| {
        let state_dir = std::env::var_os("HERDR_PLUGIN_STATE_DIR")
            .map(PathBuf::from)
            .context("HERDR_PLUGIN_STATE_DIR is not set")?;
        let run_id =
            std::env::var("HERDR_TALON_RUN_ID").context("HERDR_TALON_RUN_ID is not set")?;
        let store = RunStore::new(state_dir)?;
        let picker_result = (|| {
            let snapshot = store.claim(&run_id)?;
            let executor = ActionExecutor::new(
                herdr.clone(),
                snapshot.source_pane_id.clone(),
                snapshot.source_cwd.as_deref().map(PathBuf::from),
            );
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
    let targets = snapshot
        .targets
        .iter()
        .map(|target| target.text.clone())
        .collect::<Vec<_>>();
    let mut state = PickerState::new(snapshot.hints.clone());

    loop {
        terminal.draw(|frame| {
            frame.render_widget(TalonView::new(snapshot, &state), frame.area());
        })?;
        match state.handle_event(event::read()?, &targets) {
            InputOutcome::Continue => {}
            InputOutcome::Cancel => return Ok(()),
            InputOutcome::Complete(completion) => match executor.execute(&completion) {
                Ok(()) => return Ok(()),
                Err(error) => state.set_error(format!("Action failed: {error:#}")),
            },
        }
    }
}
