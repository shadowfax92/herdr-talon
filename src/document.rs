use std::ops::Range;

use ansi_to_tui::IntoText;
use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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
    pub column: usize,
}

impl VisualPoint {
    pub const fn new(row: usize, column: usize) -> Self {
        Self { row, column }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualSegment {
    pub row: usize,
    pub start: usize,
    pub end: usize,
}

impl VisualSegment {
    pub const fn new(row: usize, start: usize, end: usize) -> Self {
        Self { row, start, end }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRow {
    source_row: usize,
    start_column: usize,
    end_column: usize,
    start_cell: usize,
    end_cell: usize,
    line: Line<'static>,
}

impl VisualRow {
    pub fn source_row(&self) -> usize {
        self.source_row
    }

    pub fn source_columns(&self) -> Range<usize> {
        self.start_column..self.end_column
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
    terminated: Vec<bool>,
    styled_lines: Vec<Line<'static>>,
    cell_offsets: Vec<Vec<usize>>,
    width: u16,
    visual_rows: Vec<VisualRow>,
    visual_ranges: Vec<Range<usize>>,
}

impl WrappedDocument {
    pub fn new(text: &str, ansi: &str, width: u16) -> Self {
        let (lines, terminated) = logical_lines(text);
        let styled_lines = aligned_styled_lines(&lines, ansi);
        let cell_offsets = lines
            .iter()
            .map(|line| grapheme_cell_offsets(line))
            .collect();
        let mut document = Self {
            lines,
            terminated,
            styled_lines,
            cell_offsets,
            width: width.max(1),
            visual_rows: Vec::new(),
            visual_ranges: Vec::new(),
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
        self.cell_offsets
            .get(row)
            .map(|offsets| offsets.len().saturating_sub(1))
            .unwrap_or(0)
    }

    pub fn first_position(&self) -> SourcePosition {
        SourcePosition::new(0, 0)
    }

    pub fn last_line_start(&self) -> SourcePosition {
        SourcePosition::new(self.lines.len().saturating_sub(1), 0)
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
        let Some(range) = self.visual_ranges.get(position.row) else {
            return 0;
        };
        let mut last = range.start;
        for index in range.clone() {
            let row = &self.visual_rows[index];
            last = index;
            if row.start_column == row.end_column || position.column < row.end_column {
                return index;
            }
        }
        last
    }

    pub fn visual_column_for(&self, position: SourcePosition) -> usize {
        let position = self.clamp_position(position);
        let visual_row = &self.visual_rows[self.visual_row_for(position)];
        self.cell_offsets[position.row][position.column].saturating_sub(visual_row.start_cell)
    }

    pub fn position_at(&self, visual_row: usize, visual_column: usize) -> SourcePosition {
        let row = &self.visual_rows[visual_row.min(self.visual_rows.len().saturating_sub(1))];
        if row.start_column == row.end_column {
            return SourcePosition::new(row.source_row, 0);
        }
        let target_cell = row.start_cell.saturating_add(visual_column);
        let offsets = &self.cell_offsets[row.source_row];
        for column in row.start_column..row.end_column {
            let end = offsets[column.saturating_add(1)];
            if target_cell < end {
                return SourcePosition::new(row.source_row, column);
            }
        }
        SourcePosition::new(row.source_row, row.end_column.saturating_sub(1))
    }

    pub fn selected_text(
        &self,
        anchor: SourcePosition,
        cursor: SourcePosition,
        kind: SelectionKind,
    ) -> String {
        let (start, end) = ordered(self.clamp_position(anchor), self.clamp_position(cursor));
        let mut selected = String::new();
        for row in start.row..=end.row {
            let line = &self.lines[row];
            let range = selection_range(row, start, end, kind, self.line_len(row));
            selected.push_str(grapheme_slice(line, range));
            let include_terminator = match kind {
                SelectionKind::Line => self.terminated[row],
                SelectionKind::Character => row < end.row && self.terminated[row],
            };
            if include_terminator {
                selected.push('\n');
            }
        }
        selected
    }

    pub fn selection_segments(
        &self,
        anchor: SourcePosition,
        cursor: SourcePosition,
        kind: SelectionKind,
    ) -> Vec<VisualSegment> {
        let (start, end) = ordered(self.clamp_position(anchor), self.clamp_position(cursor));
        let mut segments = Vec::new();
        for source_row in start.row..=end.row {
            let selected = selection_range(source_row, start, end, kind, self.line_len(source_row));
            let offsets = &self.cell_offsets[source_row];
            for visual_index in self.visual_ranges[source_row].clone() {
                let visual = &self.visual_rows[visual_index];
                if selected.is_empty() {
                    segments.push(VisualSegment::new(visual_index, 0, 1));
                    continue;
                }
                let from_cell = offsets[selected.start].max(visual.start_cell);
                let to_cell = offsets[selected.end].min(visual.end_cell);
                if from_cell < to_cell {
                    segments.push(VisualSegment::new(
                        visual_index,
                        from_cell.saturating_sub(visual.start_cell),
                        to_cell.saturating_sub(visual.start_cell),
                    ));
                }
            }
        }
        segments
    }

    pub fn occurrence_segments(&self, occurrence: &Occurrence) -> Vec<VisualSegment> {
        let Some(range) = self.visual_ranges.get(occurrence.row) else {
            return Vec::new();
        };
        let from = occurrence.highlight_col;
        let to = from.saturating_add(occurrence.highlight_width);
        range
            .clone()
            .filter_map(|index| {
                let visual = &self.visual_rows[index];
                let start = from.max(visual.start_cell);
                let end = to.min(visual.end_cell);
                (start < end).then(|| {
                    VisualSegment::new(
                        index,
                        start.saturating_sub(visual.start_cell),
                        end.saturating_sub(visual.start_cell),
                    )
                })
            })
            .collect()
    }

    pub fn hint_anchor(&self, occurrence: &Occurrence) -> Option<VisualPoint> {
        let range = self.visual_ranges.get(occurrence.row)?;
        let cell = occurrence.hint_col;
        range
            .clone()
            .find(|index| {
                let visual = &self.visual_rows[*index];
                cell >= visual.start_cell && cell < visual.end_cell
            })
            .map(|row| {
                let visual = &self.visual_rows[row];
                VisualPoint::new(row, cell.saturating_sub(visual.start_cell))
            })
    }

    fn rebuild(&mut self) {
        self.visual_rows.clear();
        self.visual_ranges.clear();
        for (source_row, (line, styled)) in
            self.lines.iter().zip(self.styled_lines.iter()).enumerate()
        {
            let start = self.visual_rows.len();
            self.visual_rows
                .extend(wrap_line(source_row, line, styled, self.width));
            self.visual_ranges.push(start..self.visual_rows.len());
        }
    }
}

#[derive(Clone)]
struct StyledGrapheme {
    spans: Vec<Span<'static>>,
    width: usize,
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
    let graphemes = styled_graphemes(plain, styled);
    let mut rows = Vec::new();
    let mut spans = Vec::<Span<'static>>::new();
    let mut start_column = 0;
    let mut start_cell = 0usize;
    let mut cell = 0usize;

    for (column, grapheme) in graphemes.iter().enumerate() {
        if cell > start_cell
            && cell
                .saturating_sub(start_cell)
                .saturating_add(grapheme.width)
                > usize::from(width)
        {
            rows.push(visual_row(
                source_row,
                start_column,
                column,
                start_cell,
                cell,
                std::mem::take(&mut spans),
                styled,
            ));
            start_column = column;
            start_cell = cell;
        }
        push_grapheme(&mut spans, grapheme);
        cell = cell.saturating_add(grapheme.width);
    }

    rows.push(visual_row(
        source_row,
        start_column,
        graphemes.len(),
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
    start_column: usize,
    end_column: usize,
    start_cell: usize,
    end_cell: usize,
    spans: Vec<Span<'static>>,
    source: &Line<'static>,
) -> VisualRow {
    VisualRow {
        source_row,
        start_column,
        end_column,
        start_cell,
        end_cell,
        line: Line {
            style: source.style,
            alignment: source.alignment,
            spans,
        },
    }
}

fn styled_graphemes(plain: &str, line: &Line<'static>) -> Vec<StyledGrapheme> {
    let mut characters = line
        .spans
        .iter()
        .flat_map(|span| {
            span.content
                .chars()
                .map(|character| (character, span.style))
        })
        .collect::<Vec<_>>()
        .into_iter();
    plain
        .graphemes(true)
        .map(|grapheme| {
            let mut spans = Vec::new();
            for _ in grapheme.chars() {
                let (character, style) = characters.next().expect("aligned styled line");
                push_styled_character(&mut spans, character, style);
            }
            StyledGrapheme {
                spans,
                width: UnicodeWidthStr::width(grapheme),
            }
        })
        .collect()
}

fn push_grapheme(spans: &mut Vec<Span<'static>>, grapheme: &StyledGrapheme) {
    for span in &grapheme.spans {
        if let Some(last) = spans.last_mut().filter(|last| last.style == span.style) {
            last.content.to_mut().push_str(span.content.as_ref());
        } else {
            spans.push(span.clone());
        }
    }
}

fn push_styled_character(spans: &mut Vec<Span<'static>>, character: char, style: Style) {
    if let Some(last) = spans.last_mut().filter(|span| span.style == style) {
        last.content.to_mut().push(character);
    } else {
        spans.push(Span::styled(character.to_string(), style));
    }
}

fn logical_lines(text: &str) -> (Vec<String>, Vec<bool>) {
    let mut lines = Vec::new();
    let mut terminated = Vec::new();
    for chunk in text.split_inclusive('\n') {
        if let Some(line) = chunk.strip_suffix('\n') {
            lines.push(line.to_owned());
            terminated.push(true);
        } else {
            lines.push(chunk.to_owned());
            terminated.push(false);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
        terminated.push(false);
    }
    (lines, terminated)
}

fn line_content(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn grapheme_cell_offsets(line: &str) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(line.len().saturating_add(1));
    let mut cell = 0usize;
    offsets.push(cell);
    for grapheme in line.graphemes(true) {
        cell = cell.saturating_add(UnicodeWidthStr::width(grapheme));
        offsets.push(cell);
    }
    offsets
}

fn grapheme_slice(line: &str, range: Range<usize>) -> &str {
    let mut boundaries = line
        .grapheme_indices(true)
        .map(|(byte, _)| byte)
        .collect::<Vec<_>>();
    boundaries.push(line.len());
    &line[boundaries[range.start]..boundaries[range.end]]
}

fn selection_range(
    row: usize,
    start: SourcePosition,
    end: SourcePosition,
    kind: SelectionKind,
    length: usize,
) -> Range<usize> {
    match kind {
        SelectionKind::Line => 0..length,
        SelectionKind::Character if start.row == end.row => {
            start.column..end.column.saturating_add(1).min(length)
        }
        SelectionKind::Character if row == start.row => start.column..length,
        SelectionKind::Character if row == end.row => 0..end.column.saturating_add(1).min(length),
        SelectionKind::Character => 0..length,
    }
}

fn ordered(first: SourcePosition, second: SourcePosition) -> (SourcePosition, SourcePosition) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
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
        assert_eq!(document.visual_rows()[0].source_columns(), 0..2);
        assert_eq!(document.visual_rows()[1].source_columns(), 2..4);
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
    fn linewise_selections_preserve_real_line_terminators() {
        let document = WrappedDocument::new("first\n\nlast", "first\n\nlast", 20);

        assert_eq!(
            document.selected_text(
                SourcePosition::new(0, 0),
                SourcePosition::new(0, 0),
                SelectionKind::Line,
            ),
            "first\n"
        );
        assert_eq!(
            document.selected_text(
                SourcePosition::new(1, 0),
                SourcePosition::new(1, 0),
                SelectionKind::Line,
            ),
            "\n"
        );
        assert_eq!(
            document.selected_text(
                SourcePosition::new(2, 0),
                SourcePosition::new(2, 0),
                SelectionKind::Line,
            ),
            "last"
        );
    }

    #[test]
    fn grapheme_clusters_are_atomic_for_wrapping_and_coordinates() {
        let family = "👨‍👩‍👧‍👦";
        let text = format!("{family} /tmp/a");
        let document = WrappedDocument::new(&text, &text, 3);
        let occurrence = Occurrence {
            row: 0,
            highlight_col: 3,
            highlight_width: 6,
            hint_col: 3,
            hint_width: 6,
        };

        assert_eq!(document.line_len(0), 8);
        assert_eq!(document.visual_rows()[0].content(), format!("{family} "));
        assert_eq!(document.visual_rows()[0].source_columns(), 0..2);
        assert_eq!(
            document.hint_anchor(&occurrence),
            Some(VisualPoint::new(1, 0))
        );
        assert_eq!(document.position_at(0, 1), SourcePosition::new(0, 0));
        assert_eq!(document.position_at(1, 0), SourcePosition::new(0, 2));
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
