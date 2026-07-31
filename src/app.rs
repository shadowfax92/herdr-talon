use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Copy,
    Paste,
    Open,
    MultiCopy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub kind: ActionKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputOutcome {
    Continue,
    Complete(Completion),
    Cancel,
}

#[derive(Clone, Debug)]
pub struct PickerState {
    hints: Vec<String>,
    input: String,
    multi_mode: bool,
    selected: Vec<usize>,
    error: Option<String>,
}

impl PickerState {
    pub fn new(hints: Vec<String>) -> Self {
        Self {
            hints,
            input: String::new(),
            multi_mode: false,
            selected: Vec::new(),
            error: None,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, targets: &[String]) -> InputOutcome {
        if key.kind != KeyEventKind::Press {
            return InputOutcome::Continue;
        }
        if key.code == KeyCode::Esc
            || matches!(key.code, KeyCode::Char('q' | 'Q'))
            || (matches!(key.code, KeyCode::Char('c' | 'C'))
                && key.modifiers.contains(KeyModifiers::CONTROL))
        {
            return InputOutcome::Cancel;
        }
        if key.code == KeyCode::Tab {
            self.input.clear();
            self.error = None;
            if !self.multi_mode {
                self.multi_mode = true;
                return InputOutcome::Continue;
            }
            if self.selected.is_empty() {
                return InputOutcome::Cancel;
            }
            let text = self
                .selected
                .iter()
                .filter_map(|index| targets.get(*index))
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            return InputOutcome::Complete(Completion {
                kind: ActionKind::MultiCopy,
                text,
            });
        }
        if key.code == KeyCode::Backspace {
            self.input.pop();
            self.error = None;
            return InputOutcome::Continue;
        }
        let KeyCode::Char(character) = key.code else {
            return InputOutcome::Continue;
        };
        if !character.is_ascii_alphabetic() {
            return InputOutcome::Continue;
        }

        self.error = None;
        self.input.push(character.to_ascii_lowercase());
        if let Some(index) = self.hints.iter().position(|hint| hint == &self.input) {
            self.input.clear();
            let Some(text) = targets.get(index).cloned() else {
                self.error = Some("Target snapshot is inconsistent".into());
                return InputOutcome::Continue;
            };
            if self.multi_mode {
                if !self.selected.contains(&index) {
                    self.selected.push(index);
                }
                return InputOutcome::Continue;
            }
            let kind = if key.modifiers.contains(KeyModifiers::CONTROL) {
                ActionKind::Open
            } else if key.modifiers.contains(KeyModifiers::SHIFT) {
                ActionKind::Paste
            } else {
                ActionKind::Copy
            };
            return InputOutcome::Complete(Completion { kind, text });
        }
        if self.hints.iter().any(|hint| hint.starts_with(&self.input)) {
            return InputOutcome::Continue;
        }
        self.input.clear();
        self.error = Some("Unknown hint".into());
        InputOutcome::Continue
    }

    pub fn handle_event(&mut self, event: Event, targets: &[String]) -> InputOutcome {
        match event {
            Event::Key(key) => self.handle_key(key, targets),
            Event::Resize(_, _) => InputOutcome::Cancel,
            _ => InputOutcome::Continue,
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn multi_mode(&self) -> bool {
        self.multi_mode
    }

    pub fn selected(&self) -> &[usize] {
        &self.selected
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    pub fn is_visible(&self, index: usize) -> bool {
        self.selected.contains(&index)
            || self.input.is_empty()
            || self
                .hints
                .get(index)
                .is_some_and(|hint| hint.starts_with(&self.input))
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    use super::*;

    fn key(character: char, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), modifiers)
    }

    fn targets() -> Vec<String> {
        vec!["alpha".into(), "beta".into(), "gamma".into()]
    }

    #[test]
    fn progressive_hint_completes_with_the_final_modifier() {
        let mut state = PickerState::new(vec!["as".into(), "ad".into(), "f".into()]);

        assert_eq!(
            state.handle_key(key('a', KeyModifiers::NONE), &targets()),
            InputOutcome::Continue
        );
        assert_eq!(state.input(), "a");
        assert!(state.is_visible(0));
        assert!(state.is_visible(1));
        assert!(!state.is_visible(2));
        assert_eq!(
            state.handle_key(key('S', KeyModifiers::SHIFT), &targets()),
            InputOutcome::Complete(Completion {
                kind: ActionKind::Paste,
                text: "alpha".into(),
            })
        );
    }

    #[test]
    fn plain_and_control_completions_choose_copy_and_open() {
        let mut state = PickerState::new(vec!["a".into(), "s".into(), "d".into()]);
        assert_eq!(
            state.handle_key(key('a', KeyModifiers::NONE), &targets()),
            InputOutcome::Complete(Completion {
                kind: ActionKind::Copy,
                text: "alpha".into(),
            })
        );

        let mut state = PickerState::new(vec!["a".into(), "s".into(), "d".into()]);
        assert_eq!(
            state.handle_key(key('s', KeyModifiers::CONTROL), &targets()),
            InputOutcome::Complete(Completion {
                kind: ActionKind::Open,
                text: "beta".into(),
            })
        );
    }

    #[test]
    fn multi_select_preserves_order_and_ignores_duplicates() {
        let mut state = PickerState::new(vec!["a".into(), "s".into(), "d".into()]);
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &targets()),
            InputOutcome::Continue
        );
        assert_eq!(
            state.handle_key(key('s', KeyModifiers::NONE), &targets()),
            InputOutcome::Continue
        );
        assert_eq!(
            state.handle_key(key('a', KeyModifiers::NONE), &targets()),
            InputOutcome::Continue
        );
        assert_eq!(
            state.handle_key(key('s', KeyModifiers::NONE), &targets()),
            InputOutcome::Continue
        );

        assert_eq!(state.selected(), &[1, 0]);
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &targets()),
            InputOutcome::Complete(Completion {
                kind: ActionKind::MultiCopy,
                text: "beta alpha".into(),
            })
        );
    }

    #[test]
    fn cancel_keys_and_resize_release_without_an_action() {
        for key in [
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            key('q', KeyModifiers::NONE),
            key('c', KeyModifiers::CONTROL),
        ] {
            let mut state = PickerState::new(vec!["a".into()]);
            assert_eq!(state.handle_key(key, &targets()), InputOutcome::Cancel);
        }
        let mut state = PickerState::new(vec!["a".into()]);
        let release = KeyEvent {
            kind: KeyEventKind::Release,
            ..key('a', KeyModifiers::NONE)
        };
        assert_eq!(
            state.handle_key(release, &targets()),
            InputOutcome::Continue
        );
        assert_eq!(
            state.handle_event(Event::Resize(100, 40), &targets()),
            InputOutcome::Cancel
        );
    }

    #[test]
    fn invalid_prefix_resets_and_action_errors_are_clearable() {
        let mut state = PickerState::new(vec!["as".into()]);
        state.set_error("pbcopy failed");

        assert_eq!(
            state.handle_key(key('z', KeyModifiers::NONE), &targets()),
            InputOutcome::Continue
        );
        assert_eq!(state.input(), "");
        assert!(state.error().unwrap().contains("Unknown hint"));
        assert_eq!(
            state.handle_key(key('a', KeyModifiers::NONE), &targets()),
            InputOutcome::Continue
        );
        assert!(state.error().is_none());
    }
}
