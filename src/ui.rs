use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
        Widget,
    },
};

use crate::{
    app::{PickerMode, PickerState},
    document::{SelectionKind, VisualSegment},
    snapshot::RunSnapshot,
};

const TARGET_COLOR: Color = Color::Rgb(218, 165, 70);

pub struct TalonView<'a> {
    snapshot: &'a RunSnapshot,
    state: &'a PickerState,
}

impl<'a> TalonView<'a> {
    pub fn new(snapshot: &'a RunSnapshot, state: &'a PickerState) -> Self {
        Self { snapshot, state }
    }
}

impl Widget for TalonView<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        render(area, buffer, self.snapshot, self.state);
    }
}

pub fn viewport_size(area: Rect) -> (u16, u16) {
    let regions = regions(area);
    (regions.source.width.max(1), regions.source.height.max(1))
}

pub fn render(area: Rect, buffer: &mut Buffer, snapshot: &RunSnapshot, state: &PickerState) {
    Clear.render(area, buffer);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(
            Line::from(" Talon · focused pane history ").style(
                Style::default()
                    .fg(TARGET_COLOR)
                    .add_modifier(Modifier::BOLD),
            ),
        );
    let regions = regions(area);
    block.render(area, buffer);

    render_source(buffer, regions.source, state);
    render_scrollbar(buffer, regions.body, state);
    render_footer(buffer, regions.footer, snapshot, state);
}

#[derive(Clone, Copy)]
struct Regions {
    body: Rect,
    source: Rect,
    footer: Rect,
}

fn regions(area: Rect) -> Regions {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let footer_height = inner.height.min(3);
    let [body, footer] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(footer_height)]).areas(inner);
    let source = Rect::new(body.x, body.y, body.width.saturating_sub(1), body.height);
    Regions {
        body,
        source,
        footer,
    }
}

fn render_source(buffer: &mut Buffer, area: Rect, state: &PickerState) {
    for (offset, row) in state
        .document()
        .visual_rows()
        .iter()
        .skip(state.top())
        .take(usize::from(area.height))
        .enumerate()
    {
        let row_area = Rect::new(
            area.x,
            area.y.saturating_add(saturating_u16(offset)),
            area.width,
            1,
        );
        row.line().clone().render(row_area, buffer);
    }

    render_targets(buffer, area, state);
    render_search(buffer, area, state);
    render_selection(buffer, area, state);
    render_cursor(buffer, area, state);
    render_hints(buffer, area, state);
}

fn render_targets(buffer: &mut Buffer, area: Rect, state: &PickerState) {
    let style = Style::default()
        .fg(TARGET_COLOR)
        .add_modifier(Modifier::UNDERLINED);
    for (target_index, target) in state.targets().iter().enumerate() {
        if state.hint_for_target(target_index).is_none() || !state.hint_is_active(target_index) {
            continue;
        }
        for occurrence in &target.occurrences {
            for segment in state.document().occurrence_segments(occurrence) {
                paint_segment(buffer, area, state.top(), segment, style);
            }
        }
    }
}

fn render_search(buffer: &mut Buffer, area: Rect, state: &PickerState) {
    let style = Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
    for matched in state.search_matches() {
        for segment in state.document().selection_segments(
            matched.start,
            matched.end,
            SelectionKind::Character,
        ) {
            paint_segment(buffer, area, state.top(), segment, style);
        }
    }
}

fn render_selection(buffer: &mut Buffer, area: Rect, state: &PickerState) {
    let Some((anchor, cursor, kind)) = state.selection() else {
        return;
    };
    let style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    for segment in state.document().selection_segments(anchor, cursor, kind) {
        paint_segment(buffer, area, state.top(), segment, style);
    }
}

fn render_cursor(buffer: &mut Buffer, area: Rect, state: &PickerState) {
    let visual_row = state.document().visual_row_for(state.cursor());
    if visual_row < state.top()
        || visual_row >= state.top().saturating_add(usize::from(area.height))
    {
        return;
    }
    let column = state.document().visual_column_for(state.cursor());
    let position = Position::new(
        area.x.saturating_add(column),
        area.y
            .saturating_add(saturating_u16(visual_row.saturating_sub(state.top()))),
    );
    if area.contains(position) {
        buffer[position].set_style(Style::default().add_modifier(Modifier::REVERSED));
    }
}

fn render_hints(buffer: &mut Buffer, area: Rect, state: &PickerState) {
    for hint in state.visible_hints() {
        if !state.hint_is_active(hint.target_index) {
            continue;
        }
        let Some(target) = state.targets().get(hint.target_index) else {
            continue;
        };
        for occurrence in &target.occurrences {
            let Some(anchor) = state.document().hint_anchor(occurrence) else {
                continue;
            };
            if anchor.row < state.top()
                || anchor.row >= state.top().saturating_add(usize::from(area.height))
            {
                continue;
            }
            let x = area.x.saturating_add(anchor.column);
            let y = area
                .y
                .saturating_add(saturating_u16(anchor.row.saturating_sub(state.top())));
            if x >= area.right() || y >= area.bottom() {
                continue;
            }
            for (index, character) in hint.label.chars().enumerate() {
                let x = x.saturating_add(saturating_u16(index));
                if x >= area.right() {
                    break;
                }
                let typed = index < state.hint_input().chars().count();
                let style = Style::default()
                    .fg(Color::Black)
                    .bg(if typed { Color::Cyan } else { TARGET_COLOR })
                    .add_modifier(Modifier::BOLD);
                buffer.set_stringn(x, y, character.to_string(), 1, style);
            }
        }
    }
}

fn paint_segment(
    buffer: &mut Buffer,
    area: Rect,
    top: usize,
    segment: VisualSegment,
    style: Style,
) {
    if segment.row < top || segment.row >= top.saturating_add(usize::from(area.height)) {
        return;
    }
    let y = area
        .y
        .saturating_add(saturating_u16(segment.row.saturating_sub(top)));
    for column in segment.start..segment.end.min(area.width) {
        let position = Position::new(area.x.saturating_add(column), y);
        if area.contains(position) {
            buffer[position].set_style(style);
        }
    }
}

fn render_scrollbar(buffer: &mut Buffer, area: Rect, state: &PickerState) {
    let total = state.document().visual_rows().len();
    if total <= state.viewport_height() || area.is_empty() {
        return;
    }
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some("│"))
        .thumb_symbol("┃")
        .track_style(Style::default().fg(Color::DarkGray))
        .thumb_style(Style::default().fg(TARGET_COLOR));
    let mut scrollbar_state = ScrollbarState::new(total)
        .position(state.top())
        .viewport_content_length(state.viewport_height());
    scrollbar.render(area, buffer, &mut scrollbar_state);
}

fn render_footer(buffer: &mut Buffer, area: Rect, snapshot: &RunSnapshot, state: &PickerState) {
    if area.is_empty() {
        return;
    }
    let (position, total) = state.visual_position();
    let mode = match state.mode() {
        PickerMode::Browse => (" BROWSE ", Color::Cyan),
        PickerMode::VisualCharacter => (" VISUAL ", Color::Magenta),
        PickerMode::VisualLine => (" VISUAL LINE ", Color::Magenta),
        PickerMode::Search => (" SEARCH ", Color::Yellow),
    };
    let limited = if snapshot.history_limited {
        " · last 1,000 lines"
    } else {
        ""
    };
    let status = Line::from(vec![
        Span::styled(
            mode.0,
            Style::default()
                .fg(Color::Black)
                .bg(mode.1)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " {position}/{total} · {} visible target{}{} ",
                state.visible_hints().len(),
                if state.visible_hints().len() == 1 {
                    ""
                } else {
                    "s"
                },
                limited
            ),
            Style::default().fg(Color::Gray),
        ),
    ]);
    render_footer_line(buffer, area, 0, status);

    let help = match state.mode() {
        PickerMode::Browse => {
            " type hint → copy & close · ↑↓/hjkl move · PgUp/PgDn · v/V select · / search · q close "
                .to_string()
        }
        PickerMode::VisualCharacter | PickerMode::VisualLine => {
            " move to extend · y copy & close · Esc cancel selection ".into()
        }
        PickerMode::Search => format!(
            " /{}█ · type query · Enter jump · Esc cancel ",
            state.search_query()
        ),
    };
    render_footer_line(
        buffer,
        area,
        1,
        Line::from(help).style(Style::default().fg(Color::White)),
    );

    let (detail, style) = if let Some(error) = state.error() {
        (
            format!(" {error} "),
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )
    } else if !state.hint_input().is_empty() {
        (
            format!(" hint: {}… ", state.hint_input()),
            Style::default().fg(TARGET_COLOR),
        )
    } else if state.mode() == PickerMode::Search {
        (
            format!(
                " {} match{} ",
                state.search_matches().len(),
                if state.search_matches().len() == 1 {
                    ""
                } else {
                    "es"
                }
            ),
            Style::default().fg(Color::DarkGray),
        )
    } else {
        (
            " g/G top/bottom · Ctrl-u/d half page · n/N previous/next search match ".into(),
            Style::default().fg(Color::DarkGray),
        )
    };
    render_footer_line(buffer, area, 2, Line::from(detail).style(style));
}

fn render_footer_line(buffer: &mut Buffer, area: Rect, offset: u16, line: Line<'_>) {
    if offset >= area.height {
        return;
    }
    line.render(
        Rect::new(area.x, area.y.saturating_add(offset), area.width, 1),
        buffer,
    );
}

fn saturating_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::{config::PatternDefinition, matcher::find_targets};

    use super::*;

    fn snapshot(history_limited: bool) -> RunSnapshot {
        let text = "red TKT-1000\nplain second line\nneedle TKT-2000";
        let patterns = vec![PatternDefinition {
            name: "ticket".into(),
            regex: "TKT-[0-9]+".into(),
        }];
        RunSnapshot {
            source_pane_id: "w1:p1".into(),
            text: text.into(),
            ansi: "\u{1b}[31mred\u{1b}[0m TKT-1000\nplain second line\nneedle TKT-2000".into(),
            history_limited,
            targets: find_targets(text, &patterns).unwrap(),
            alphabet: vec!['a', 's', 'd'],
        }
    }

    fn buffer_text(buffer: &Buffer) -> String {
        (buffer.area.y..buffer.area.bottom())
            .map(|y| {
                (buffer.area.x..buffer.area.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_styled_history_hints_scrollbar_and_clear_instructions() {
        let snapshot = snapshot(true);
        let area = Rect::new(0, 0, 80, 12);
        let mut state = PickerState::new(&snapshot).unwrap();
        let (width, height) = viewport_size(area);
        state.set_viewport(width, height);
        let mut buffer = Buffer::empty(area);

        render(area, &mut buffer, &snapshot, &state);

        let text = buffer_text(&buffer);
        assert!(text.contains("Talon · focused pane history"));
        assert!(text.contains("type hint → copy & close"));
        assert!(text.contains("last 1,000 lines"));
        assert!(buffer.content.iter().any(|cell| cell.fg == Color::Red));
        assert!(buffer
            .content
            .iter()
            .any(|cell| cell.symbol() == "a" && cell.bg == TARGET_COLOR));
    }

    #[test]
    fn visual_and_search_modes_have_distinct_feedback() {
        let snapshot = snapshot(false);
        let area = Rect::new(0, 0, 72, 12);
        let mut state = PickerState::new(&snapshot).unwrap();
        let (width, height) = viewport_size(area);
        state.set_viewport(width, height);
        state.handle_key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        let mut buffer = Buffer::empty(area);

        render(area, &mut buffer, &snapshot, &state);

        assert!(buffer_text(&buffer).contains("y copy & close"));
        assert!(buffer.content.iter().any(|cell| cell.bg == Color::Cyan));

        state.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        state.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for character in "needle".chars() {
            state.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        render(area, &mut buffer, &snapshot, &state);

        assert!(buffer_text(&buffer).contains("/needle█"));
        assert!(buffer.content.iter().any(|cell| cell.fg == Color::Magenta));
    }
}
