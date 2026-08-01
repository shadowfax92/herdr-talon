use std::path::PathBuf;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event};

use crate::actions::ActionExecutor;
use crate::app::{InputOutcome, PickerState};
use crate::herdr::Herdr;
use crate::snapshot::{RunSnapshot, RunStore};
use crate::ui::TalonView;

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
            let executor = ActionExecutor::new(
                herdr.clone(),
                snapshot.source_pane_id.clone(),
                snapshot.source_cwd.as_deref().map(PathBuf::from),
            );
            ratatui::run(|terminal| run_picker(terminal, &snapshot, &executor, &herdr))
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
    herdr: &Herdr,
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
        let event = event::read()?;
        if matches!(event, Event::Resize(_, _)) {
            terminal.autoresize()?;
            let current = herdr.layout(&snapshot.source_pane_id)?;
            if tab_geometry_changed(snapshot, &current) {
                return Ok(());
            }
            continue;
        }
        match state.handle_event(event, &targets) {
            InputOutcome::Continue => {}
            InputOutcome::Cancel => return Ok(()),
            InputOutcome::Complete(completion) => match executor.execute(&completion) {
                Ok(()) => return Ok(()),
                Err(error) => state.set_error(format!("Action failed: {error:#}")),
            },
        }
    }
}

fn tab_geometry_changed(snapshot: &RunSnapshot, current: &crate::herdr::Layout) -> bool {
    snapshot.layout.area != current.area
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_tab_geometry_change_cancels_the_picker() {
        let captured = crate::herdr::Layout {
            workspace_id: "w1".into(),
            tab_id: "w1:t1".into(),
            zoomed: false,
            area: crate::herdr::Rect {
                x: 30,
                y: 1,
                width: 100,
                height: 30,
            },
            focused_pane_id: "w1:p1".into(),
            panes: Vec::new(),
        };
        let snapshot = RunSnapshot {
            source_pane_id: "w1:p1".into(),
            source_cwd: None,
            layout: captured.clone(),
            panes: Vec::new(),
            targets: Vec::new(),
            hints: Vec::new(),
        };
        let mut current = captured;

        current.zoomed = true;
        assert!(!tab_geometry_changed(&snapshot, &current));

        current.area.width = 120;
        assert!(tab_geometry_changed(&snapshot, &current));
    }
}
