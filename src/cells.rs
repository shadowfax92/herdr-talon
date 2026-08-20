use ratatui::buffer::CellWidth;
use unicode_segmentation::UnicodeSegmentation;

pub fn width(text: &str) -> usize {
    text.graphemes(true)
        .map(|grapheme| usize::from(grapheme.cell_width()))
        .sum()
}
