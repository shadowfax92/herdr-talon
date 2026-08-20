use std::ops::Range;

use anyhow::Result;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    document::{SelectionKind, SourcePosition, WrappedDocument},
    hints::generate_hints,
    matcher::Target,
    snapshot::RunSnapshot,
};

const MAX_SEARCH_MATCHES: usize = 50_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputOutcome {
    Continue,
    Complete(Completion),
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerMode {
    Browse,
    VisualCharacter,
    VisualLine,
    Search,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleHint {
    pub target_index: usize,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    pub start: SourcePosition,
    pub end: SourcePosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Browse,
    Visual {
        anchor: SourcePosition,
        kind: SelectionKind,
    },
    Search,
}

#[derive(Clone, Debug)]
pub struct PickerState {
    document: WrappedDocument,
    targets: Vec<Target>,
    alphabet: Vec<char>,
    mode: Mode,
    cursor: SourcePosition,
    desired_column: usize,
    top: usize,
    viewport_height: usize,
    initialized: bool,
    visible_hints: Vec<VisibleHint>,
    hint_input: String,
    committed_search: String,
    search_input: String,
    search_origin: Option<SourcePosition>,
    search_matches: Vec<SearchMatch>,
    search_row_ranges: Vec<Range<usize>>,
    search_limited: bool,
    error: Option<String>,
}

impl PickerState {
    pub fn new(snapshot: &RunSnapshot) -> Result<Self> {
        generate_hints(&snapshot.alphabet, 1)?;
        let document = WrappedDocument::deferred(&snapshot.text, &snapshot.ansi);
        let cursor = document.last_position();
        let line_count = document.lines().len();
        Ok(Self {
            document,
            targets: snapshot.targets.clone(),
            alphabet: snapshot.alphabet.clone(),
            mode: Mode::Browse,
            cursor,
            desired_column: 0,
            top: 0,
            viewport_height: 1,
            initialized: false,
            visible_hints: Vec::new(),
            hint_input: String::new(),
            committed_search: String::new(),
            search_input: String::new(),
            search_origin: None,
            search_matches: Vec::new(),
            search_row_ranges: vec![0..0; line_count],
            search_limited: false,
            error: None,
        })
    }

    pub fn set_viewport(&mut self, width: u16, height: u16) {
        let width = width.max(1);
        let height = usize::from(height.max(1));
        let width_changed = self.document.width() != width;
        let height_changed = self.viewport_height != height;
        let previous_top = self.top;
        let was_initialized = self.initialized;
        self.document.reflow(width);
        self.viewport_height = height;
        if self.initialized {
            if width_changed {
                self.cursor = self.document.clamp_position(self.cursor);
                self.desired_column = self.document.visual_column_for(self.cursor);
            }
            self.ensure_cursor_visible();
        } else {
            self.cursor = self.document.last_position();
            self.top = self
                .document
                .visual_rows()
                .len()
                .saturating_sub(self.viewport_height);
            self.desired_column = self.document.visual_column_for(self.cursor);
            self.initialized = true;
        }
        if !was_initialized || width_changed || height_changed || self.top != previous_top {
            self.hint_input.clear();
            self.refresh_hints();
        }
    }

    pub fn handle_event(&mut self, event: Event) -> InputOutcome {
        match event {
            Event::Key(key) => self.handle_key(key),
            _ => InputOutcome::Continue,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputOutcome {
        if key.kind != KeyEventKind::Press {
            return InputOutcome::Continue;
        }
        if matches!(key.code, KeyCode::Char('c' | 'C'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return InputOutcome::Cancel;
        }
        if matches!(self.mode, Mode::Search) {
            return self.handle_search_key(key);
        }
        if key.code == KeyCode::Esc {
            if matches!(self.mode, Mode::Visual { .. }) {
                self.mode = Mode::Browse;
                self.error = None;
                return InputOutcome::Continue;
            }
            if !self.hint_input.is_empty() {
                self.hint_input.clear();
                self.error = None;
                return InputOutcome::Continue;
            }
            return InputOutcome::Cancel;
        }
        if matches!(key.code, KeyCode::Char('q' | 'Q')) {
            return InputOutcome::Cancel;
        }
        if matches!(self.mode, Mode::Visual { .. }) && matches!(key.code, KeyCode::Char('y' | 'Y'))
        {
            let text = self.selected_text().unwrap_or_default();
            if text.is_empty() {
                self.error = Some("Selection is empty".into());
                return InputOutcome::Continue;
            }
            return InputOutcome::Complete(Completion { text });
        }
        if matches!(self.mode, Mode::Browse) {
            match key.code {
                KeyCode::Char('v') if key.modifiers.is_empty() => {
                    self.start_selection(SelectionKind::Character);
                    return InputOutcome::Continue;
                }
                KeyCode::Char('V') | KeyCode::Char('v')
                    if key.modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    self.start_selection(SelectionKind::Line);
                    return InputOutcome::Continue;
                }
                KeyCode::Char('/') => {
                    self.start_search();
                    return InputOutcome::Continue;
                }
                KeyCode::Char('n') if key.modifiers.is_empty() => {
                    self.repeat_search(true);
                    return InputOutcome::Continue;
                }
                KeyCode::Char('N') | KeyCode::Char('n')
                    if key.modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    self.repeat_search(false);
                    return InputOutcome::Continue;
                }
                _ => {}
            }
        }
        if self.handle_motion(key) {
            return InputOutcome::Continue;
        }
        if matches!(self.mode, Mode::Browse) && key.code == KeyCode::Backspace {
            self.hint_input.pop();
            self.error = None;
            return InputOutcome::Continue;
        }
        if matches!(self.mode, Mode::Browse) {
            if let KeyCode::Char(character) = key.code {
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
                {
                    let character = character.to_ascii_lowercase();
                    if self.alphabet.contains(&character) {
                        return self.handle_hint(character);
                    }
                }
            }
        }
        InputOutcome::Continue
    }

    pub fn mode(&self) -> PickerMode {
        match self.mode {
            Mode::Browse => PickerMode::Browse,
            Mode::Visual {
                kind: SelectionKind::Character,
                ..
            } => PickerMode::VisualCharacter,
            Mode::Visual {
                kind: SelectionKind::Line,
                ..
            } => PickerMode::VisualLine,
            Mode::Search => PickerMode::Search,
        }
    }

    pub fn document(&self) -> &WrappedDocument {
        &self.document
    }

    pub fn targets(&self) -> &[Target] {
        &self.targets
    }

    pub fn cursor(&self) -> SourcePosition {
        self.cursor
    }

    pub fn top(&self) -> usize {
        self.top
    }

    pub fn viewport_height(&self) -> usize {
        self.viewport_height
    }

    pub fn visible_hints(&self) -> &[VisibleHint] {
        &self.visible_hints
    }

    pub fn hint_is_active(&self, label: &str) -> bool {
        self.hint_input.is_empty() || label.starts_with(&self.hint_input)
    }

    pub fn hint_input(&self) -> &str {
        &self.hint_input
    }

    pub fn selection(&self) -> Option<(SourcePosition, SourcePosition, SelectionKind)> {
        let Mode::Visual { anchor, kind } = self.mode else {
            return None;
        };
        Some((anchor, self.cursor, kind))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection()
            .map(|(anchor, cursor, kind)| self.document.selected_text(anchor, cursor, kind))
    }

    pub fn search_query(&self) -> &str {
        if matches!(self.mode, Mode::Search) {
            &self.search_input
        } else {
            &self.committed_search
        }
    }

    pub fn search_matches(&self) -> &[SearchMatch] {
        &self.search_matches
    }

    pub fn search_matches_in(&self, row: usize, columns: Range<usize>) -> &[SearchMatch] {
        let Some(row_range) = self.search_row_ranges.get(row) else {
            return &[];
        };
        let matches = &self.search_matches[row_range.clone()];
        let start = matches.partition_point(|matched| matched.end.column < columns.start);
        let end = matches.partition_point(|matched| matched.start.column < columns.end);
        &matches[start.min(end)..end]
    }

    pub fn search_limited(&self) -> bool {
        self.search_limited
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    pub fn visual_position(&self) -> (usize, usize) {
        (
            self.document.visual_row_for(self.cursor).saturating_add(1),
            self.document.visual_rows().len(),
        )
    }

    fn handle_motion(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_vertical(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_vertical(1),
            KeyCode::Left | KeyCode::Char('h') => self.move_left(),
            KeyCode::Right | KeyCode::Char('l') => self.move_right(),
            KeyCode::Home | KeyCode::Char('0') => self.move_line_start(),
            KeyCode::End | KeyCode::Char('$') => self.move_line_end(),
            KeyCode::PageUp => self.move_page(-1),
            KeyCode::PageDown => self.move_page(1),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_visual(-((self.viewport_height / 2).max(1) as isize))
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_visual((self.viewport_height / 2).max(1) as isize)
            }
            KeyCode::Char('g') if key.modifiers.is_empty() => {
                self.move_to(self.document.first_position())
            }
            KeyCode::Char('G') | KeyCode::Char('g')
                if key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                self.move_to(self.document.last_position())
            }
            _ => return false,
        }
        true
    }

    fn move_vertical(&mut self, delta: isize) {
        if matches!(
            self.mode,
            Mode::Visual {
                kind: SelectionKind::Line,
                ..
            }
        ) {
            let row = self
                .cursor
                .row
                .saturating_add_signed(delta)
                .min(self.document.lines().len().saturating_sub(1));
            self.cursor = SourcePosition::new(
                row,
                self.cursor
                    .column
                    .min(self.document.line_len(row).saturating_sub(1)),
            );
            self.after_motion(true);
        } else {
            self.move_visual(delta);
        }
    }

    fn move_visual(&mut self, delta: isize) {
        let current = self.document.visual_row_for(self.cursor);
        let last = self.document.visual_rows().len().saturating_sub(1);
        let target = current.saturating_add_signed(delta).min(last);
        self.cursor = self.document.position_at(target, self.desired_column);
        self.after_motion(false);
    }

    fn move_page(&mut self, direction: isize) {
        let distance = self.viewport_height.max(1);
        let delta = direction.saturating_mul(distance as isize);
        let current = self.document.visual_row_for(self.cursor);
        let last = self.document.visual_rows().len().saturating_sub(1);
        let target = current.saturating_add_signed(delta).min(last);
        self.cursor = self.document.position_at(target, self.desired_column);
        let previous_top = self.top;
        self.top = self
            .top
            .saturating_add_signed(delta)
            .min(self.document.visual_rows().len().saturating_sub(distance));
        self.hint_input.clear();
        self.error = None;
        if self.top != previous_top {
            self.refresh_hints();
        }
    }

    fn move_left(&mut self) {
        self.cursor.column = self.cursor.column.saturating_sub(1);
        self.after_motion(true);
    }

    fn move_right(&mut self) {
        self.cursor.column = self
            .cursor
            .column
            .saturating_add(1)
            .min(self.document.line_len(self.cursor.row).saturating_sub(1));
        self.after_motion(true);
    }

    fn move_line_start(&mut self) {
        self.cursor.column = 0;
        self.after_motion(true);
    }

    fn move_line_end(&mut self) {
        self.cursor.column = self.document.line_len(self.cursor.row).saturating_sub(1);
        self.after_motion(true);
    }

    fn move_to(&mut self, position: SourcePosition) {
        self.cursor = self.document.clamp_position(position);
        self.after_motion(true);
    }

    fn after_motion(&mut self, update_desired_column: bool) {
        if update_desired_column {
            self.desired_column = self.document.visual_column_for(self.cursor);
        }
        self.hint_input.clear();
        self.error = None;
        let previous_top = self.top;
        self.ensure_cursor_visible();
        if self.top != previous_top {
            self.refresh_hints();
        }
    }

    fn ensure_cursor_visible(&mut self) {
        let cursor_row = self.document.visual_row_for(self.cursor);
        if cursor_row < self.top {
            self.top = cursor_row;
        } else if cursor_row >= self.top.saturating_add(self.viewport_height) {
            self.top = cursor_row
                .saturating_add(1)
                .saturating_sub(self.viewport_height);
        }
        self.top = self.top.min(
            self.document
                .visual_rows()
                .len()
                .saturating_sub(self.viewport_height),
        );
    }

    fn start_selection(&mut self, kind: SelectionKind) {
        self.mode = Mode::Visual {
            anchor: self.cursor,
            kind,
        };
        self.hint_input.clear();
        self.error = None;
    }

    fn start_search(&mut self) {
        self.mode = Mode::Search;
        self.search_origin = Some(self.cursor);
        self.search_input.clear();
        self.search_matches.clear();
        self.search_row_ranges.fill(0..0);
        self.search_limited = false;
        self.hint_input.clear();
        self.error = None;
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> InputOutcome {
        match key.code {
            KeyCode::Esc => {
                if let Some(origin) = self.search_origin.take() {
                    self.cursor = origin;
                }
                self.mode = Mode::Browse;
                self.search_input.clear();
                self.rebuild_search_matches(false);
                self.after_motion(true);
            }
            KeyCode::Enter => {
                if !self.search_input.is_empty() && self.search_matches.is_empty() {
                    return InputOutcome::Continue;
                }
                if !self.search_input.is_empty() {
                    self.committed_search.clone_from(&self.search_input);
                }
                self.mode = Mode::Browse;
                self.search_origin = None;
                self.rebuild_search_matches(false);
                self.error = None;
            }
            KeyCode::Backspace => {
                self.search_input.pop();
                self.rebuild_search_matches(true);
            }
            KeyCode::Char(character)
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                ) && !character.is_control() =>
            {
                self.search_input.push(character);
                self.rebuild_search_matches(true);
            }
            _ => {}
        }
        InputOutcome::Continue
    }

    fn rebuild_search_matches(&mut self, preview: bool) {
        let query = if matches!(self.mode, Mode::Search) {
            self.search_input.as_str()
        } else {
            self.committed_search.as_str()
        };
        let results = find_search_matches(self.document.lines(), query);
        self.search_matches = results.matches;
        self.search_limited = results.limited;
        self.search_row_ranges =
            index_search_rows(self.document.lines().len(), &self.search_matches);
        if query.is_empty() {
            self.error = None;
            return;
        }
        if self.search_matches.is_empty() {
            self.error = Some(format!("No matches for /{query}"));
            return;
        }
        self.error = None;
        if preview {
            let origin = self.search_origin.unwrap_or(self.cursor);
            let origin_row = SourcePosition::new(origin.row, 0);
            let index = self
                .search_matches
                .partition_point(|matched| matched.start < origin_row);
            let next = self
                .search_matches
                .get(index)
                .unwrap_or(&self.search_matches[0]);
            self.cursor = next.start;
            self.desired_column = self.document.visual_column_for(self.cursor);
            let previous_top = self.top;
            self.ensure_cursor_visible();
            if self.top != previous_top {
                self.refresh_hints();
            }
        }
    }

    fn repeat_search(&mut self, forward: bool) {
        if self.committed_search.is_empty() {
            return;
        }
        self.rebuild_search_matches(false);
        if self.search_matches.is_empty() {
            return;
        }
        let index = if forward {
            self.search_matches
                .partition_point(|matched| matched.start <= self.cursor)
                % self.search_matches.len()
        } else {
            self.search_matches
                .partition_point(|matched| matched.start < self.cursor)
                .checked_sub(1)
                .unwrap_or(self.search_matches.len() - 1)
        };
        self.cursor = self.search_matches[index].start;
        self.after_motion(true);
    }

    fn handle_hint(&mut self, character: char) -> InputOutcome {
        self.error = None;
        self.hint_input.push(character);
        if let Some(hint) = self
            .visible_hints
            .iter()
            .find(|hint| hint.label == self.hint_input)
        {
            let text = self.targets[hint.target_index].text.clone();
            self.hint_input.clear();
            return InputOutcome::Complete(Completion { text });
        }
        if self
            .visible_hints
            .iter()
            .any(|hint| hint.label.starts_with(&self.hint_input))
        {
            return InputOutcome::Continue;
        }
        self.hint_input.clear();
        self.error = Some("Unknown hint".into());
        InputOutcome::Continue
    }

    fn refresh_hints(&mut self) {
        let bottom = self.top.saturating_add(self.viewport_height);
        let target_indices = self
            .targets
            .iter()
            .enumerate()
            .filter_map(|(index, target)| {
                target
                    .occurrences
                    .iter()
                    .any(|occurrence| {
                        self.document
                            .hint_anchor_in_viewport(occurrence, self.top, bottom)
                            .is_some()
                    })
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        let labels = generate_hints(&self.alphabet, target_indices.len()).unwrap_or_default();
        self.visible_hints = target_indices
            .into_iter()
            .zip(labels)
            .map(|(target_index, label)| VisibleHint {
                target_index,
                label,
            })
            .collect();
    }
}

struct SearchResults {
    matches: Vec<SearchMatch>,
    limited: bool,
}

fn find_search_matches(lines: &[String], query: &str) -> SearchResults {
    if query.is_empty() {
        return SearchResults {
            matches: Vec::new(),
            limited: false,
        };
    }
    let mut matches = Vec::new();
    let mut limited = false;
    'lines: for (row, line) in lines.iter().enumerate() {
        let boundaries = line
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .collect::<Vec<_>>();
        for (byte, matched) in line.match_indices(query) {
            if matches.len() == MAX_SEARCH_MATCHES {
                limited = true;
                break 'lines;
            }
            let start = boundaries
                .partition_point(|boundary| *boundary <= byte)
                .saturating_sub(1);
            let end_byte = byte.saturating_add(matched.len());
            let end = boundaries
                .partition_point(|boundary| *boundary < end_byte)
                .saturating_sub(1)
                .max(start);
            matches.push(SearchMatch {
                start: SourcePosition::new(row, start),
                end: SourcePosition::new(row, end),
            });
        }
    }
    SearchResults { matches, limited }
}

fn index_search_rows(line_count: usize, matches: &[SearchMatch]) -> Vec<Range<usize>> {
    let mut ranges = vec![0..0; line_count];
    let mut index = 0;
    for (row, range) in ranges.iter_mut().enumerate() {
        let start = index;
        while index < matches.len() && matches[index].start.row == row {
            index += 1;
        }
        *range = start..index;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::{config::PatternDefinition, matcher::find_targets, snapshot::RunSnapshot};

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified(character: char, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), modifiers)
    }

    fn snapshot(text: &str) -> RunSnapshot {
        let patterns = vec![PatternDefinition {
            name: "ticket".into(),
            regex: "TKT-[0-9]+".into(),
        }];
        RunSnapshot {
            source_pane_id: "w1:p1".into(),
            text: text.into(),
            ansi: text.into(),
            targets: find_targets(text, &patterns).unwrap(),
            alphabet: vec!['a', 's', 'd'],
        }
    }

    #[test]
    fn opens_at_newest_output_and_supports_copy_mode_motions() {
        let text = (0..10)
            .map(|row| format!("line {row}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = PickerState::new(&snapshot(&text)).unwrap();
        state.set_viewport(20, 3);

        assert_eq!(state.top(), 7);
        assert_eq!(state.cursor().row, 9);
        assert_eq!(state.cursor().column, 5);

        state.handle_key(key(KeyCode::Char('k')));
        assert_eq!(state.cursor().row, 8);
        state.handle_key(key(KeyCode::PageUp));
        assert_eq!(state.cursor().row, 5);
        state.handle_key(modified('u', KeyModifiers::CONTROL));
        assert_eq!(state.cursor().row, 4);
        state.handle_key(key(KeyCode::Char('g')));
        assert_eq!(state.cursor().row, 0);
        state.handle_key(modified('G', KeyModifiers::SHIFT));
        assert_eq!(state.cursor().row, 9);
    }

    #[test]
    fn visible_hints_reassign_after_navigation_and_copy_the_target() {
        let mut state = PickerState::new(&snapshot("TKT-1000\nplain\nTKT-2000\nTKT-3000")).unwrap();
        state.set_viewport(20, 2);

        assert_eq!(
            state
                .visible_hints()
                .iter()
                .map(|hint| (hint.target_index, hint.label.as_str()))
                .collect::<Vec<_>>(),
            vec![(1, "a"), (2, "s")]
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char('a'))),
            InputOutcome::Complete(Completion {
                text: "TKT-2000".into()
            })
        );
        assert!(state.hint_input().is_empty());

        let mut state = PickerState::new(&snapshot("TKT-1000\nplain\nTKT-2000\nTKT-3000")).unwrap();
        state.set_viewport(20, 2);
        state.handle_key(key(KeyCode::Char('a')));
        state.handle_key(key(KeyCode::PageUp));

        assert!(state.hint_input().is_empty());
        assert_eq!(state.visible_hints()[0].target_index, 0);
        assert_eq!(state.visible_hints()[0].label, "a");
    }

    #[test]
    fn visual_selection_copies_exact_text_across_soft_and_logical_wraps() {
        let mut state = PickerState::new(&snapshot("abcdef\nsecond")).unwrap();
        state.set_viewport(3, 5);
        state.handle_key(key(KeyCode::Char('g')));
        state.handle_key(key(KeyCode::Char('l')));
        state.handle_key(key(KeyCode::Char('v')));
        for _ in 0..3 {
            state.handle_key(key(KeyCode::Char('l')));
        }

        assert_eq!(state.selected_text().as_deref(), Some("bcde"));
        assert_eq!(
            state.handle_key(key(KeyCode::Char('y'))),
            InputOutcome::Complete(Completion {
                text: "bcde".into()
            })
        );

        let mut state = PickerState::new(&snapshot("abcdef\nsecond")).unwrap();
        state.set_viewport(3, 5);
        state.handle_key(key(KeyCode::Char('g')));
        state.handle_key(modified('V', KeyModifiers::SHIFT));
        state.handle_key(key(KeyCode::Char('j')));

        assert_eq!(state.selected_text().as_deref(), Some("abcdef\nsecond"));
    }

    #[test]
    fn resize_preserves_logical_selection_and_reflows_in_place() {
        let mut state = PickerState::new(&snapshot("abcdefghij")).unwrap();
        state.set_viewport(6, 3);
        state.handle_key(key(KeyCode::Char('g')));
        state.handle_key(key(KeyCode::Char('l')));
        state.handle_key(key(KeyCode::Char('v')));
        for _ in 0..6 {
            state.handle_key(key(KeyCode::Char('l')));
        }
        let selection = state.selection().unwrap();

        state.set_viewport(3, 3);

        assert_eq!(state.selection().unwrap(), selection);
        assert_eq!(state.selected_text().as_deref(), Some("bcdefgh"));
        assert_eq!(state.document().width(), 3);
    }

    #[test]
    fn repeated_draws_preserve_the_sticky_visual_column() {
        let mut state = PickerState::new(&snapshot("abcdef\nx\nabcdef")).unwrap();
        state.set_viewport(20, 3);
        state.handle_key(key(KeyCode::Char('g')));
        for _ in 0..5 {
            state.handle_key(key(KeyCode::Char('l')));
        }

        state.handle_key(key(KeyCode::Char('j')));
        assert_eq!(state.cursor(), SourcePosition::new(1, 0));
        state.set_viewport(20, 3);
        state.handle_key(key(KeyCode::Char('j')));

        assert_eq!(state.cursor(), SourcePosition::new(2, 5));
    }

    #[test]
    fn resize_clears_a_partial_viewport_hint() {
        let mut run = snapshot("TKT-1000 TKT-2000 TKT-3000 TKT-4000 TKT-5000");
        run.alphabet = vec!['a', 's'];
        let mut state = PickerState::new(&run).unwrap();
        state.set_viewport(80, 2);
        let prefix = state
            .visible_hints()
            .iter()
            .find(|hint| hint.label.len() > 1)
            .unwrap()
            .label
            .chars()
            .next()
            .unwrap();

        assert_eq!(
            state.handle_key(key(KeyCode::Char(prefix))),
            InputOutcome::Continue
        );
        assert!(!state.hint_input().is_empty());
        state.set_viewport(40, 2);

        assert!(state.hint_input().is_empty());
    }

    #[test]
    fn newest_cursor_is_visible_when_the_final_logical_line_wraps() {
        let mut state = PickerState::new(&snapshot("older\nabcdefghij")).unwrap();
        state.set_viewport(3, 2);
        let visual_row = state.document().visual_row_for(state.cursor());

        assert_eq!(state.cursor(), SourcePosition::new(1, 9));
        assert!(visual_row >= state.top());
        assert!(visual_row < state.top() + state.viewport_height());

        state.handle_key(key(KeyCode::Char('g')));
        state.handle_key(modified('G', KeyModifiers::SHIFT));
        assert_eq!(state.cursor(), SourcePosition::new(1, 9));
    }

    #[test]
    fn document_wrapping_is_deferred_until_the_viewport_is_known() {
        let text = "x".repeat(100_000);
        let mut state = PickerState::new(&snapshot(&text)).unwrap();

        assert!(state.document().visual_rows().is_empty());
        state.set_viewport(100, 20);
        assert_eq!(state.document().visual_rows().len(), 1_000);
    }

    #[test]
    fn a_wrapped_target_keeps_its_hint_when_its_start_scrolls_above_view() {
        let text = "/abcdefghij";
        let patterns = vec![PatternDefinition {
            name: "path".into(),
            regex: r"/[a-z]+".into(),
        }];
        let run = RunSnapshot {
            source_pane_id: "w1:p1".into(),
            text: text.into(),
            ansi: text.into(),
            targets: find_targets(text, &patterns).unwrap(),
            alphabet: vec!['a', 's'],
        };
        let mut state = PickerState::new(&run).unwrap();

        state.set_viewport(4, 1);

        assert_eq!(state.visible_hints().len(), 1);
        assert_eq!(state.visible_hints()[0].target_index, 0);
    }

    #[test]
    fn modified_hint_keys_do_not_copy_or_change_the_prefix() {
        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
        ] {
            let mut state = PickerState::new(&snapshot("TKT-1000")).unwrap();
            state.set_viewport(20, 2);

            assert_eq!(
                state.handle_key(modified('a', modifiers)),
                InputOutcome::Continue
            );
            assert!(state.hint_input().is_empty());
        }
    }

    #[test]
    fn incremental_search_and_repeat_wrap_in_both_directions() {
        let mut state =
            PickerState::new(&snapshot("alpha\nneedle one\nmiddle\nneedle two")).unwrap();
        state.set_viewport(20, 2);
        state.handle_key(key(KeyCode::Char('/')));
        for character in "needle".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }

        assert_eq!(state.mode(), PickerMode::Search);
        assert_eq!(state.cursor().row, 3);
        assert_eq!(state.search_query(), "needle");
        state.handle_key(key(KeyCode::Enter));
        state.handle_key(key(KeyCode::Char('n')));
        assert_eq!(state.cursor().row, 1);
        state.handle_key(modified('N', KeyModifiers::SHIFT));
        assert_eq!(state.cursor().row, 3);
    }

    #[test]
    fn search_positions_use_grapheme_columns() {
        let family = "👨‍👩‍👧‍👦";
        let mut state = PickerState::new(&snapshot(&format!("{family} /tmp/a"))).unwrap();
        state.set_viewport(20, 2);
        state.handle_key(key(KeyCode::Char('/')));
        for character in "/tmp".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }

        assert_eq!(state.cursor(), SourcePosition::new(0, 2));
    }

    #[test]
    fn enter_keeps_a_zero_match_search_open_with_its_error() {
        let mut state = PickerState::new(&snapshot("haystack")).unwrap();
        state.set_viewport(20, 2);
        state.handle_key(key(KeyCode::Char('/')));
        for character in "needle".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }

        state.handle_key(key(KeyCode::Enter));

        assert_eq!(state.mode(), PickerMode::Search);
        assert!(state.error().unwrap().contains("No matches"));
    }

    #[test]
    fn dense_searches_are_bounded_and_indexed_by_visible_source_range() {
        let text = "x".repeat(MAX_SEARCH_MATCHES + 100);
        let mut state = PickerState::new(&snapshot(&text)).unwrap();
        state.set_viewport(100, 2);
        state.handle_key(key(KeyCode::Char('/')));
        state.handle_key(key(KeyCode::Char('x')));
        let visual = &state.document().visual_rows()[state.top()];

        assert_eq!(state.search_matches().len(), MAX_SEARCH_MATCHES);
        assert!(state.search_limited());
        assert_eq!(
            state
                .search_matches_in(visual.source_row(), visual.source_columns())
                .len(),
            100
        );
    }

    #[test]
    fn escape_cancels_modes_before_closing_the_popup() {
        let mut state = PickerState::new(&snapshot("abcdef")).unwrap();
        state.set_viewport(20, 2);
        state.handle_key(key(KeyCode::Char('v')));
        assert_eq!(state.handle_key(key(KeyCode::Esc)), InputOutcome::Continue);
        assert_eq!(state.mode(), PickerMode::Browse);
        assert_eq!(state.handle_key(key(KeyCode::Esc)), InputOutcome::Cancel);
    }
}
