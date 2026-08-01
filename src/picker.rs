use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use ratatui::crossterm::event::{self, Event};

use crate::actions::ActionExecutor;
use crate::app::{InputOutcome, PickerState};
use crate::herdr::Herdr;
use crate::snapshot::{RunSnapshot, RunStore};
use crate::ui::TalonView;

const STARTUP_RESIZE_QUIET_PERIOD: Duration = Duration::from_millis(50);

trait EventReader {
    fn poll(&mut self, timeout: Duration) -> std::io::Result<bool>;
    fn read(&mut self) -> std::io::Result<Event>;
}

struct CrosstermEvents;

impl EventReader for CrosstermEvents {
    fn poll(&mut self, timeout: Duration) -> std::io::Result<bool> {
        event::poll(timeout)
    }

    fn read(&mut self) -> std::io::Result<Event> {
        event::read()
    }
}

fn drain_startup_resizes<E, F>(events: &mut E, mut autoresize: F) -> Result<VecDeque<Event>>
where
    E: EventReader,
    F: FnMut() -> Result<()>,
{
    let mut pending = VecDeque::new();
    while events.poll(STARTUP_RESIZE_QUIET_PERIOD)? {
        match events.read()? {
            Event::Resize(_, _) => autoresize()?,
            event => pending.push_back(event),
        }
    }
    Ok(pending)
}

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
    let mut events = CrosstermEvents;
    let mut pending_events = drain_startup_resizes(&mut events, || {
        terminal.autoresize()?;
        Ok(())
    })?;

    loop {
        terminal.draw(|frame| {
            frame.render_widget(TalonView::new(snapshot, &state), frame.area());
        })?;
        let event = match pending_events.pop_front() {
            Some(event) => event,
            None => events.read()?,
        };
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    struct FakeEvents {
        events: VecDeque<Event>,
        timeouts: Vec<Duration>,
    }

    impl EventReader for FakeEvents {
        fn poll(&mut self, timeout: Duration) -> std::io::Result<bool> {
            self.timeouts.push(timeout);
            Ok(!self.events.is_empty())
        }

        fn read(&mut self) -> std::io::Result<Event> {
            self.events
                .pop_front()
                .ok_or_else(|| std::io::Error::other("no event"))
        }
    }

    #[test]
    fn startup_resizes_are_drained_without_dropping_the_first_input() {
        let key = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        let mut events = FakeEvents {
            events: VecDeque::from([
                Event::FocusGained,
                Event::Resize(120, 72),
                key.clone(),
                Event::Resize(475, 70),
            ]),
            timeouts: Vec::new(),
        };
        let mut autoresizes = 0;

        let pending = drain_startup_resizes(&mut events, || {
            autoresizes += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(autoresizes, 2);
        assert_eq!(pending, VecDeque::from([Event::FocusGained, key]));
        assert_eq!(events.timeouts, vec![STARTUP_RESIZE_QUIET_PERIOD; 5]);
    }
}
