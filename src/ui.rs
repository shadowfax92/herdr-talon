use ansi_to_tui::IntoText;
use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::Text,
    widgets::{Block, Borders, Widget},
};

use crate::app::PickerState;
use crate::snapshot::RunSnapshot;

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

pub fn render(area: Rect, buffer: &mut Buffer, snapshot: &RunSnapshot, state: &PickerState) {
    buffer.reset();
    let mut source_content = None;
    for layout_pane in &snapshot.layout.panes {
        let Some(pane) = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == layout_pane.pane_id)
        else {
            continue;
        };
        let outer = normalize_rect(area, &snapshot.layout.area, &layout_pane.rect);
        if outer.is_empty() {
            continue;
        }
        let (content, bordered) = content_rect(outer, pane.viewport_rows);
        if bordered {
            let color = if layout_pane.focused {
                Color::Magenta
            } else {
                Color::DarkGray
            };
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color))
                .render(outer, buffer);
        }
        let text = pane
            .ansi
            .as_bytes()
            .into_text()
            .unwrap_or_else(|_| Text::raw(pane.text.clone()));
        (&text).render(content, buffer);
        if pane.pane_id == snapshot.source_pane_id {
            source_content = Some(content);
        }
    }

    if let Some(content) = source_content {
        render_targets(buffer, content, snapshot, state);
    }
    render_status(buffer, area, state);
}

fn normalize_rect(area: Rect, layout_area: &crate::herdr::Rect, pane: &crate::herdr::Rect) -> Rect {
    let x = area.x.saturating_add(pane.x.saturating_sub(layout_area.x));
    let y = area.y.saturating_add(pane.y.saturating_sub(layout_area.y));
    if x >= area.right() || y >= area.bottom() {
        return Rect::new(x, y, 0, 0);
    }
    Rect::new(
        x,
        y,
        pane.width.min(area.right().saturating_sub(x)),
        pane.height.min(area.bottom().saturating_sub(y)),
    )
}

fn content_rect(outer: Rect, viewport_rows: u16) -> (Rect, bool) {
    let bordered = viewport_rows > 0 && outer.height.saturating_sub(viewport_rows) >= 2;
    let inset = u16::from(bordered);
    let available_height = outer.height.saturating_sub(inset.saturating_mul(2));
    let height = if viewport_rows == 0 {
        available_height
    } else {
        viewport_rows.min(available_height)
    };
    let chrome_width = if bordered { 3 } else { 1 };
    (
        Rect::new(
            outer.x.saturating_add(inset),
            outer.y.saturating_add(inset),
            outer.width.saturating_sub(chrome_width),
            height,
        ),
        bordered,
    )
}

fn render_targets(buffer: &mut Buffer, content: Rect, snapshot: &RunSnapshot, state: &PickerState) {
    for (index, target) in snapshot.targets.iter().enumerate() {
        if !state.is_visible(index) {
            continue;
        }
        let selected = state.selected().contains(&index);
        let highlight = if selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Yellow)
        };
        let hint = if selected {
            highlight
        } else {
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD)
        };
        for occurrence in &target.occurrences {
            let y = content.y.saturating_add(occurrence.row);
            let start = content.x.saturating_add(occurrence.highlight_col);
            let end = start
                .saturating_add(occurrence.highlight_width)
                .min(content.right());
            if y >= content.bottom() {
                continue;
            }
            for x in start..end {
                let position = Position::new(x, y);
                if buffer.area.contains(position) {
                    buffer[position].set_style(highlight);
                }
            }
            let hint_x = content.x.saturating_add(occurrence.hint_col);
            if hint_x < content.right() && buffer.area.contains(Position::new(hint_x, y)) {
                let max_width = usize::from(content.right().saturating_sub(hint_x));
                if let Some(value) = snapshot.hints.get(index) {
                    buffer.set_stringn(hint_x, y, value, max_width, hint);
                }
            }
        }
    }
}

fn render_status(buffer: &mut Buffer, area: Rect, state: &PickerState) {
    let message = state.error().map(str::to_string).or_else(|| {
        state
            .multi_mode()
            .then(|| format!(" MULTI {} · Tab copies ", state.selected().len()))
    });
    let Some(message) = message else {
        return;
    };
    let y = area.bottom().saturating_sub(1);
    let style = if state.error().is_some() {
        Style::default()
            .fg(Color::White)
            .bg(Color::Red)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    };
    buffer.set_stringn(area.x, y, message, usize::from(area.width), style);
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::{Color, Modifier};

    use crate::herdr::{Layout, LayoutPane, Rect as HerdrRect};
    use crate::matcher::{Occurrence, Target};
    use crate::snapshot::PaneSnapshot;

    use super::*;

    fn snapshot() -> RunSnapshot {
        RunSnapshot {
            source_pane_id: "w1:p1".into(),
            source_cwd: None,
            layout: Layout {
                workspace_id: "w1".into(),
                tab_id: "w1:t1".into(),
                zoomed: false,
                area: HerdrRect {
                    x: 30,
                    y: 1,
                    width: 12,
                    height: 3,
                },
                focused_pane_id: "w1:p1".into(),
                panes: vec![LayoutPane {
                    pane_id: "w1:p1".into(),
                    focused: true,
                    rect: HerdrRect {
                        x: 30,
                        y: 1,
                        width: 12,
                        height: 3,
                    },
                }],
            },
            panes: vec![PaneSnapshot {
                pane_id: "w1:p1".into(),
                viewport_rows: 3,
                text: "X deadbeef\n".into(),
                ansi: "\u{1b}[31mX deadbeef\u{1b}[0m\n".into(),
            }],
            targets: vec![Target {
                text: "deadbeef".into(),
                occurrences: vec![Occurrence {
                    row: 0,
                    highlight_col: 2,
                    highlight_width: 8,
                    hint_col: 2,
                    hint_width: 8,
                }],
            }],
            hints: vec!["a".into()],
        }
    }

    #[test]
    fn ansi_backdrop_and_hint_layers_render_at_normalized_coordinates() {
        let snapshot = snapshot();
        let state = PickerState::new(snapshot.hints.clone());
        let area = Rect::new(0, 0, 12, 3);
        let mut buffer = Buffer::empty(area);

        render(area, &mut buffer, &snapshot, &state);

        assert_eq!(buffer[(0, 0)].symbol(), "X");
        assert_eq!(buffer[(0, 0)].fg, Color::Red);
        assert_eq!(buffer[(2, 0)].symbol(), "a");
        assert_eq!(buffer[(2, 0)].fg, Color::Green);
        assert!(buffer[(2, 0)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(3, 0)].fg, Color::Yellow);
    }

    #[test]
    fn multi_selected_targets_use_selected_style() {
        let snapshot = snapshot();
        let mut state = PickerState::new(snapshot.hints.clone());
        let targets = snapshot
            .targets
            .iter()
            .map(|target| target.text.clone())
            .collect::<Vec<_>>();
        state.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &targets);
        state.handle_key(
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
            &targets,
        );
        let area = Rect::new(0, 0, 12, 3);
        let mut buffer = Buffer::empty(area);

        render(area, &mut buffer, &snapshot, &state);

        assert_eq!(buffer[(2, 0)].fg, Color::Cyan);
        assert_eq!(buffer[(3, 0)].fg, Color::Cyan);
    }

    #[test]
    fn border_and_gap_geometry_uses_viewport_rows_for_content_inset() {
        let mut snapshot = snapshot();
        snapshot.layout.area = HerdrRect {
            x: 30,
            y: 1,
            width: 14,
            height: 5,
        };
        snapshot.layout.panes[0].rect = HerdrRect {
            x: 30,
            y: 1,
            width: 14,
            height: 5,
        };
        snapshot.panes[0].viewport_rows = 3;
        let state = PickerState::new(snapshot.hints.clone());
        let area = Rect::new(0, 0, 14, 5);
        let mut buffer = Buffer::empty(area);

        render(area, &mut buffer, &snapshot, &state);

        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(1, 1)].symbol(), "X");
        assert_eq!(buffer[(3, 1)].symbol(), "a");
    }
}
