//! Compact, model-readable terminal capture with style runs.

use std::fmt::Write;

use crate::types::{CellData, ScreenContent};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Style {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    inverse: bool,
    dim: bool,
}

impl Style {
    fn from_cell(cell: &CellData) -> Self {
        Self {
            fg: cell.fg.as_deref().and_then(color_to_hex),
            bg: cell.bg.as_deref().and_then(color_to_hex),
            bold: cell.bold,
            italic: cell.italic,
            underline: cell.underline,
            strikethrough: cell.strikethrough,
            inverse: cell.inverse,
            dim: cell.faint,
        }
    }

    fn is_default(&self) -> bool {
        self == &Self::default()
    }

    fn write_to(&self, out: &mut String) {
        if let Some(fg) = &self.fg {
            write!(out, " fg={fg}").unwrap();
        }
        if let Some(bg) = &self.bg {
            write!(out, " bg={bg}").unwrap();
        }
        for (set, name) in [
            (self.bold, "bold"),
            (self.italic, "italic"),
            (self.underline, "underline"),
            (self.strikethrough, "strikethrough"),
            (self.inverse, "inverse"),
            (self.dim, "dim"),
        ] {
            if set {
                write!(out, " {name}").unwrap();
            }
        }
    }
}

/// Format a styled capture.
///
/// Row text is JSON-escaped. Style ranges are half-open captured-cell indices;
/// default cells are omitted and adjacent cells with the same style are one run.
/// Rows with no text and no styled cells are omitted.
pub fn format(screen: &ScreenContent) -> String {
    let mut out = format!(
        "screen {}x{}\ncursor {},{} {}",
        screen.cols,
        screen.rows,
        screen.cursor.x,
        screen.cursor.y,
        if screen.cursor.visible {
            "visible"
        } else {
            "hidden"
        }
    );

    for row in &screen.screen {
        let text = if row.cells.is_empty() {
            row.text.trim_end().to_string()
        } else {
            let end = row
                .cells
                .iter()
                .rposition(|cell| cell.grapheme != " " || !Style::from_cell(cell).is_default())
                .map_or(0, |index| index + 1);
            row.cells[..end]
                .iter()
                .map(|cell| cell.grapheme.as_str())
                .collect()
        };
        let mut runs = String::new();
        let mut start = 0;
        while start < row.cells.len() {
            let style = Style::from_cell(&row.cells[start]);
            let mut end = start + 1;
            while end < row.cells.len() && Style::from_cell(&row.cells[end]) == style {
                end += 1;
            }
            if !style.is_default() {
                write!(runs, "\n  {start}..{end}").unwrap();
                style.write_to(&mut runs);
            }
            start = end;
        }
        // Blank, unstyled rows carry nothing; omitting them keeps a mostly
        // empty screen cheap to read.
        if text.is_empty() && runs.is_empty() {
            continue;
        }
        write!(
            out,
            "\nrow {}: {}{runs}",
            row.row,
            serde_json::to_string(&text).expect("serializing a string cannot fail")
        )
        .unwrap();
    }

    out
}

fn color_to_hex(color: &str) -> Option<String> {
    if let Some(hex) = color.strip_prefix('#')
        && hex.len() == 6
        && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Some(format!("#{}", hex.to_ascii_lowercase()));
    }
    let index: u8 = color.strip_prefix("palette:")?.parse().ok()?;
    let (r, g, b) = palette_to_rgb(index);
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

fn palette_to_rgb(index: u8) -> (u8, u8, u8) {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (170, 0, 0),
        (0, 170, 0),
        (170, 85, 0),
        (0, 0, 170),
        (170, 0, 170),
        (0, 170, 170),
        (170, 170, 170),
        (85, 85, 85),
        (255, 85, 85),
        (85, 255, 85),
        (255, 255, 85),
        (85, 85, 255),
        (255, 85, 255),
        (85, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let value = index - 16;
            (
                CUBE[(value / 36) as usize],
                CUBE[((value / 6) % 6) as usize],
                CUBE[(value % 6) as usize],
            )
        }
        232..=255 => {
            let value = 8 + (index - 232) * 10;
            (value, value, value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CursorInfo, CursorStyle, RowContent};

    fn cell(grapheme: &str) -> CellData {
        CellData {
            grapheme: grapheme.into(),
            fg: None,
            bg: None,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            inverse: false,
            faint: false,
        }
    }

    #[test]
    fn formats_only_non_default_style_runs() {
        let mut red_bold = cell("A");
        red_bold.fg = Some("palette:1".into());
        red_bold.bold = true;
        let mut all_attributes = cell("C");
        all_attributes.bg = Some("#A1B2C3".into());
        all_attributes.italic = true;
        all_attributes.underline = true;
        all_attributes.strikethrough = true;
        all_attributes.inverse = true;
        all_attributes.faint = true;
        let screen = ScreenContent {
            cols: 8,
            rows: 3,
            cursor: CursorInfo {
                x: 3,
                y: 1,
                visible: false,
                style: CursorStyle::Block,
            },
            title: None,
            screen: vec![
                RowContent {
                    row: 0,
                    text: String::new(),
                    cells: vec![red_bold.clone(), red_bold, cell(" "), all_attributes],
                },
                RowContent {
                    row: 1,
                    text: "   ".into(),
                    cells: vec![],
                },
                RowContent {
                    row: 2,
                    text: "quote: \"   ".into(),
                    cells: vec![],
                },
            ],
        };

        assert_eq!(
            format(&screen),
            "screen 8x3\ncursor 3,1 hidden\nrow 0: \"AA C\"\n  0..2 fg=#aa0000 bold\n  3..4 bg=#a1b2c3 italic underline strikethrough inverse dim\nrow 2: \"quote: \\\"\""
        );
    }
}
