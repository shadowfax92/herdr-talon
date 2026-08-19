use std::ops::Range;

use ansi_to_tui::IntoText;
use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

use crate::matcher::Occurrence;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourcePosition {
    pub row: usize,
    pub column: usize,
}

impl SourcePosition {
    pub const fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    Character,
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualPoint {
    pub row: usize,
    pub column: u16,
}

impl VisualPoint {
    pub const fn new(row: usize, column: u16) -> Self {
        Self { row, column }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualSegment {
    pub row: usize,
    pub start: u16,
    pub end: u16,
}

impl VisualSegment {
    pub const fn new(row: usize, start: u16, end: u16) -> Self {
        Self { row, start, end }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRow {
    source_row: usize,
    start_char: usize,
    end_char: usize,
    start_cell: usize,
    end_cell: usize,
    line: Line<'static>,
}

impl VisualRow {
    pub fn source_row(&self) -> usize {
        self.source_row
    }

    pub fn source_chars(&self) -> Range<usize> {
        self.start_char..self.end_char
    }

    pub fn source_cells(&self) -> Range<usize> {
        self.start_cell..self.end_cell
    }

    pub fn line(&self) -> &Line<'static> {
        &self.line
    }

    pub fn content(&self) -> String {
        self.line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct WrappedDocument {
    lines: Vec<String>,
    styled_lines: Vec<Line<'static>>,
    width: u16,
    visual_rows: Vec<VisualRow>,
}

impl WrappedDocument {
    pub fn new(text: &str, ansi: &str, width: u16) -> Self {
        let lines = logical_lines(text);
        let styled_lines = aligned_styled_lines(&lines, ansi);
        let mut document = Self {
            lines,
            styled_lines,
            width: width.max(1),
            visual_rows: Vec::new(),
        };
        document.rebuild();
        document
    }

    pub fn reflow(&mut self, width: u16) {
        let width = width.max(1);
        if width == self.width {
            return;
        }
        self.width = width;
        self.rebuild();
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn visual_rows(&self) -> &[VisualRow] {
        &self.visual_rows
    }

    pub fn line_len(&self, row: usize) -> usize {
        self.lines
            .get(row)
            .map(|line| line.chars().count())
            .unwrap_or(0)
    }

    pub fn first_position(&self) -> SourcePosition {
        SourcePosition::new(0, 0)
    }

    pub fn last_position(&self) -> SourcePosition {
        let row = self.lines.len().saturating_sub(1);
        SourcePosition::new(row, self.line_len(row).saturating_sub(1))
    }

    pub fn clamp_position(&self, position: SourcePosition) -> SourcePosition {
        let row = position.row.min(self.lines.len().saturating_sub(1));
        SourcePosition::new(
            row,
            position.column.min(self.line_len(row).saturating_sub(1)),
        )
    }

    pub fn visual_row_for(&self, position: SourcePosition) -> usize {
        let position = self.clamp_position(position);
        let mut last = 0;
        for (index, row) in self.visual_rows.iter().enumerate() {
            if row.source_row != position.row {
                continue;
            }
            last = index;
            if row.start_char == row.end_char || position.column < row.end_char {
                return index;
            }
        }
        last
    }

    pub fn visual_column_for(&self, position: SourcePosition) -> u16 {
        let position = self.clamp_position(position);
        let visual_row = &self.visual_rows[self.visual_row_for(position)];
        let cell = cell_at_char(&self.lines[position.row], position.column);
        saturating_u16(cell.saturating_sub(visual_row.start_cell))
    }

    pub fn position_at(&self, visual_row: usize, visual_column: u16) -> SourcePosition {
        let row = &self.visual_rows[visual_row.min(self.visual_rows.len().saturating_sub(1))];
        if row.start_char == row.end_char {
            return SourcePosition::new(row.source_row, 0);
        }
        let target_cell = row.start_cell.saturating_add(usize::from(visual_column));
        let line = &self.lines[row.source_row];
        for column in row.start_char..row.end_char {
            let end = cell_at_char(line, column.saturating_add(1));
            if target_cell < end {
                return SourcePosition::new(row.source_row, column);
            }
        }
        SourcePosition::new(row.source_row, row.end_char.saturating_sub(1))
    }

    pub fn selected_text(
        &self,
        anchor: SourcePosition,
        cursor: SourcePosition,
        kind: SelectionKind,
    ) -> String {
        let (start, end) = ordered(self.clamp_position(anchor), self.clamp_position(cursor));
        let mut selected = Vec::with_capacity(end.row.saturating_sub(start.row).saturating_add(1));
        for row in start.row..=end.row {
            let line = &self.lines[row];
            let length = line.chars().count();
            let (from, to) = match kind {
                SelectionKind::Line => (0, length),
                SelectionKind::Character if start.row == end.row => {
                    (start.column, end.column.saturating_add(1).min(length))
                }
                SelectionKind::Character if row == start.row => (start.column, length),
                SelectionKind::Character if row == end.row => {
                    (0, end.column.saturating_add(1).min(length))
                }
                SelectionKind::Character => (0, length),
            };
            selected.push(char_slice(line, from, to));
        }
        selected.join("\n")
    }

    pub fn selection_segments(
        &self,
        anchor: SourcePosition,
        cursor: SourcePosition,
        kind: SelectionKind,
    ) -> Vec<VisualSegment> {
        let (start, end) = ordered(self.clamp_position(anchor), self.clamp_position(cursor));
        let mut segments = Vec::new();
        for (visual_index, visual) in self.visual_rows.iter().enumerate() {
            if visual.source_row < start.row || visual.source_row > end.row {
                continue;
            }
            let line = &self.lines[visual.source_row];
            let length = line.chars().count();
            let (from, to) = match kind {
                SelectionKind::Line => (0, length),
                SelectionKind::Character if start.row == end.row => {
                    (start.column, end.column.saturating_add(1).min(length))
                }
                SelectionKind::Character if visual.source_row == start.row => {
                    (start.column, length)
                }
                SelectionKind::Character if visual.source_row == end.row => {
                    (0, end.column.saturating_add(1).min(length))
                }
                SelectionKind::Character => (0, length),
            };
            if length == 0 && from == 0 && to == 0 {
                segments.push(VisualSegment::new(visual_index, 0, 1));
                continue;
            }
            let from_cell = cell_at_char(line, from).max(visual.start_cell);
            let to_cell = cell_at_char(line, to).min(visual.end_cell);
            if from_cell < to_cell {
                segments.push(VisualSegment::new(
                    visual_index,
                    saturating_u16(from_cell.saturating_sub(visual.start_cell)),
                    saturating_u16(to_cell.saturating_sub(visual.start_cell)),
                ));
            }
        }
        segments
    }

    pub fn occurrence_segments(&self, occurrence: &Occurrence) -> Vec<VisualSegment> {
        let source_row = usize::from(occurrence.row);
        let from = usize::from(occurrence.highlight_col);
        let to = from.saturating_add(usize::from(occurrence.highlight_width));
        self.visual_rows
            .iter()
            .enumerate()
            .filter_map(|(index, visual)| {
                if visual.source_row != source_row {
                    return None;
                }
                let start = from.max(visual.start_cell);
                let end = to.min(visual.end_cell);
                (start < end).then(|| {
                    VisualSegment::new(
                        index,
                        saturating_u16(start.saturating_sub(visual.start_cell)),
                        saturating_u16(end.saturating_sub(visual.start_cell)),
                    )
                })
            })
            .collect()
    }

    pub fn hint_anchor(&self, occurrence: &Occurrence) -> Option<VisualPoint> {
        let source_row = usize::from(occurrence.row);
        let cell = usize::from(occurrence.hint_col);
        self.visual_rows
            .iter()
            .enumerate()
            .find(|(_, visual)| {
                visual.source_row == source_row
                    && cell >= visual.start_cell
                    && cell < visual.end_cell
            })
            .map(|(row, visual)| {
                VisualPoint::new(row, saturating_u16(cell.saturating_sub(visual.start_cell)))
            })
    }

    fn rebuild(&mut self) {
        self.visual_rows.clear();
        for (source_row, (line, styled)) in
            self.lines.iter().zip(self.styled_lines.iter()).enumerate()
        {
            self.visual_rows
                .extend(wrap_line(source_row, line, styled, self.width));
        }
    }
}

#[derive(Clone, Copy)]
struct StyledCharacter {
    character: char,
    style: Style,
}

fn aligned_styled_lines(lines: &[String], ansi: &str) -> Vec<Line<'static>> {
    let parsed = ansi.as_bytes().into_text().ok();
    lines
        .iter()
        .enumerate()
        .map(|(row, plain)| {
            parsed
                .as_ref()
                .and_then(|text| text.lines.get(row))
                .filter(|line| line_content(line) == *plain)
                .cloned()
                .unwrap_or_else(|| Line::raw(plain.clone()))
        })
        .collect()
}

fn wrap_line(source_row: usize, plain: &str, styled: &Line<'static>, width: u16) -> Vec<VisualRow> {
    let characters = styled_characters(styled);
    let mut rows = Vec::new();
    let mut spans = Vec::<Span<'static>>::new();
    let mut start_char = 0;
    let mut start_cell = 0usize;
    let mut cell = 0usize;

    for (column, styled_char) in characters.iter().enumerate() {
        let character_width = UnicodeWidthChar::width(styled_char.character).unwrap_or(0);
        if cell > start_cell
            && cell
                .saturating_sub(start_cell)
                .saturating_add(character_width)
                > usize::from(width)
        {
            rows.push(visual_row(
                source_row,
                start_char,
                column,
                start_cell,
                cell,
                std::mem::take(&mut spans),
                styled,
            ));
            start_char = column;
            start_cell = cell;
        }
        push_character(&mut spans, *styled_char);
        cell = cell.saturating_add(character_width);
    }

    rows.push(visual_row(
        source_row,
        start_char,
        characters.len(),
        start_cell,
        cell,
        spans,
        styled,
    ));
    if plain.is_empty() {
        rows[0].line = Line {
            style: styled.style,
            alignment: styled.alignment,
            spans: vec![Span::raw(String::new())],
        };
    }
    rows
}

fn visual_row(
    source_row: usize,
    start_char: usize,
    end_char: usize,
    start_cell: usize,
    end_cell: usize,
    spans: Vec<Span<'static>>,
    source: &Line<'static>,
) -> VisualRow {
    VisualRow {
        source_row,
        start_char,
        end_char,
        start_cell,
        end_cell,
        line: Line {
            style: source.style,
            alignment: source.alignment,
            spans,
        },
    }
}

fn styled_characters(line: &Line<'static>) -> Vec<StyledCharacter> {
    line.spans
        .iter()
        .flat_map(|span| {
            span.content.chars().map(|character| StyledCharacter {
                character,
                style: span.style,
            })
        })
        .collect()
}

fn push_character(spans: &mut Vec<Span<'static>>, styled: StyledCharacter) {
    if let Some(last) = spans.last_mut().filter(|span| span.style == styled.style) {
        last.content.to_mut().push(styled.character);
    } else {
        spans.push(Span::styled(styled.character.to_string(), styled.style));
    }
}

fn logical_lines(text: &str) -> Vec<String> {
    let mut lines = text.split('\n').map(str::to_owned).collect::<Vec<_>>();
    if text.ends_with('\n') && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn line_content(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn cell_at_char(line: &str, column: usize) -> usize {
    line.chars()
        .take(column)
        .map(|character| UnicodeWidthChar::width(character).unwrap_or(0))
        .sum()
}

fn char_slice(line: &str, from: usize, to: usize) -> String {
    line.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect()
}

fn ordered(first: SourcePosition, second: SourcePosition) -> (SourcePosition, SourcePosition) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use crate::matcher::Occurrence;

    use super::*;

    #[test]
    fn wraps_at_terminal_cells_and_keeps_empty_logical_lines() {
        let document = WrappedDocument::new("ab界c\n\nxyz", "ab界c\n\nxyz", 3);

        assert_eq!(
            document
                .visual_rows()
                .iter()
                .map(VisualRow::content)
                .collect::<Vec<_>>(),
            vec!["ab", "界c", "", "xyz"]
        );
        assert_eq!(document.visual_rows()[0].source_chars(), 0..2);
        assert_eq!(document.visual_rows()[1].source_chars(), 2..4);
        assert_eq!(document.visual_rows()[1].source_cells(), 2..5);
        assert_eq!(document.visual_rows()[2].source_row(), 1);
    }

    #[test]
    fn preserves_aligned_ansi_styles_and_falls_back_for_mismatched_rows() {
        let styled = WrappedDocument::new("abcdef", "\u{1b}[31mabcdef\u{1b}[0m", 3);
        assert_eq!(
            styled.visual_rows()[0].line().spans[0].style.fg,
            Some(Color::Red)
        );
        assert_eq!(
            styled.visual_rows()[1].line().spans[0].style.fg,
            Some(Color::Red)
        );

        let mismatched = WrappedDocument::new("abcdef", "\u{1b}[31mother\u{1b}[0m", 3);
        assert_eq!(mismatched.visual_rows()[0].content(), "abc");
        assert_eq!(mismatched.visual_rows()[0].line().spans[0].style.fg, None);
    }

    #[test]
    fn reflow_preserves_source_positions() {
        let mut document = WrappedDocument::new("abcdefgh", "abcdefgh", 4);
        let position = SourcePosition::new(0, 5);

        assert_eq!(document.visual_row_for(position), 1);
        document.reflow(3);

        assert_eq!(document.visual_row_for(position), 1);
        assert_eq!(document.visual_column_for(position), 2);
        assert_eq!(document.position_at(1, 2), position);
    }

    #[test]
    fn selections_copy_logical_text_without_soft_wrap_newlines() {
        let document = WrappedDocument::new("abcdef\nsecond", "abcdef\nsecond", 3);

        assert_eq!(
            document.selected_text(
                SourcePosition::new(0, 1),
                SourcePosition::new(0, 4),
                SelectionKind::Character,
            ),
            "bcde"
        );
        assert_eq!(
            document.selected_text(
                SourcePosition::new(0, 4),
                SourcePosition::new(1, 2),
                SelectionKind::Character,
            ),
            "ef\nsec"
        );
        assert_eq!(
            document.selected_text(
                SourcePosition::new(0, 3),
                SourcePosition::new(1, 1),
                SelectionKind::Line,
            ),
            "abcdef\nsecond"
        );
    }

    #[test]
    fn occurrences_project_across_wrapped_rows() {
        let document = WrappedDocument::new("abcdefghij", "abcdefghij", 4);
        let occurrence = Occurrence {
            row: 0,
            highlight_col: 2,
            highlight_width: 5,
            hint_col: 2,
            hint_width: 5,
        };

        assert_eq!(
            document.occurrence_segments(&occurrence),
            vec![VisualSegment::new(0, 2, 4), VisualSegment::new(1, 0, 3),]
        );
        assert_eq!(
            document.hint_anchor(&occurrence),
            Some(VisualPoint::new(0, 2))
        );
    }
}
