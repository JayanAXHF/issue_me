use std::str::FromStr;

use rat_widget::{
    event::{HandleEvent, Outcome, Regular},
    focus::{FocusFlag, HasFocus},
};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{Event, KeyCode},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Widget},
};

use crate::ui::COLOR_PROFILE;

const HUES: [(&str, [&str; 5]); 8] = [
    ("Red", ["ffebe9", "ffcecb", "ffaba8", "ff8182", "fa4549"]),
    ("Orange", ["fff8c5", "ffec99", "f7c843", "e16f24", "bc4c00"]),
    ("Yellow", ["fff8c5", "fae17d", "eac54f", "d4a72c", "bf8700"]),
    ("Green", ["dafbe1", "aceebb", "6fdd8b", "4ac26b", "2da44e"]),
    ("Teal", ["d2f4ea", "96e9da", "4ac9b0", "1ea7a1", "0a7f7f"]),
    ("Blue", ["ddf4ff", "b6e3ff", "80ccff", "54aeff", "0969da"]),
    ("Purple", ["fbefff", "ecd8ff", "d8b9ff", "c297ff", "a475f9"]),
    ("Gray", ["f6f8fa", "eaeef2", "d0d7de", "8c959f", "57606a"]),
];
const HUE_KEYS: [&str; 8] = ["R", "O", "Y", "G", "T", "B", "P", "K"];

#[derive(Debug, Clone)]
pub struct ColorPickerState {
    row: usize,
    col: usize,
    area: Rect,
    pub rat_focus: Option<FocusFlag>,
}

impl Default for ColorPickerState {
    fn default() -> Self {
        Self {
            row: 7,
            col: 2,
            area: Rect::default(),
            rat_focus: Some(FocusFlag::new().with_name("label_color_picker")),
        }
    }
}

impl ColorPickerState {
    pub fn with_initial_hex(hex: &str) -> Self {
        let normalized = hex.trim().trim_start_matches('#').to_ascii_lowercase();
        for (r, (_, shades)) in HUES.iter().enumerate() {
            for (c, shade) in shades.iter().enumerate() {
                if normalized == *shade {
                    return Self {
                        row: r,
                        col: c,
                        ..Self::default()
                    };
                }
            }
        }
        Self::default()
    }

    pub fn selected_hex(&self) -> &'static str {
        HUES[self.row].1[self.col]
    }

    pub fn set_area(&mut self, area: Rect) {
        self.area = area;
    }
}

impl HandleEvent<Event, Regular, Outcome> for ColorPickerState {
    fn handle(&mut self, event: &Event, _: Regular) -> Outcome {
        if !self.is_focused() {
            return Outcome::Continue;
        }
        let Event::Key(key) = event else {
            return Outcome::Continue;
        };
        match key.code {
            KeyCode::Up => {
                if self.row > 0 {
                    self.row -= 1;
                    return Outcome::Changed;
                }
            }
            KeyCode::Down => {
                if self.row + 1 < HUES.len() {
                    self.row += 1;
                    return Outcome::Changed;
                }
            }
            KeyCode::Left => {
                if self.col > 0 {
                    self.col -= 1;
                    return Outcome::Changed;
                }
            }
            KeyCode::Right => {
                if self.col + 1 < HUES[0].1.len() {
                    self.col += 1;
                    return Outcome::Changed;
                }
            }
            _ => {}
        }
        Outcome::Continue
    }
}

impl HasFocus for ColorPickerState {
    fn build(&self, builder: &mut rat_widget::focus::FocusBuilder) {
        builder.leaf_widget(self);
    }

    fn area(&self) -> Rect {
        self.area
    }

    fn focus(&self) -> FocusFlag {
        self.rat_focus
            .clone()
            .unwrap_or_else(|| FocusFlag::new().with_name("label_color_picker"))
    }
}

#[derive(Debug, Default)]
pub struct ColorPicker;

impl ColorPicker {
    pub fn render(&self, area: Rect, buf: &mut Buffer, state: &mut ColorPickerState) {
        state.set_area(area);
        Clear.render(area, buf);
        let mut block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .title("Color picker");
        if state.is_focused() {
            block = block.border_style(Style::default().yellow());
        }
        let inner = block.inner(area);
        block.render(area, buf);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let grid_area = sections[0];
        let info_area = sections[1];

        let mut lines = Vec::with_capacity(HUES.len());
        for (row_idx, ((_, shades), key)) in HUES.iter().zip(HUE_KEYS).enumerate() {
            let mut spans = vec![Span::styled(
                format!("{key} "),
                Style::default().add_modifier(Modifier::BOLD),
            )];
            for (col_idx, shade) in shades.iter().enumerate() {
                let bg = parse_hex_color(shade);
                let is_selected = row_idx == state.row && col_idx == state.col;
                let text = if is_selected { "<>" } else { "  " };
                let mut style = Style::default().bg(bg);
                if is_selected {
                    style = style.fg(Color::Black).bold();
                }
                spans.push(Span::raw("  "));
                spans.push(Span::styled(text, style));
            }
            lines.push(Line::from(spans));
        }
        Paragraph::new(lines).render(grid_area, buf);

        let selected = state.selected_hex();
        let preview = parse_hex_color(selected);
        let info = Line::from(vec![
            Span::styled(" ", Style::default().bg(preview)),
            Span::raw(format!(" #{selected}")),
        ]);
        Paragraph::new(info).render(info_area, buf);
    }
}

fn parse_hex_color(hex: &str) -> Color {
    let mut c = Color::from_str(&format!("#{hex}")).unwrap_or(Color::Gray);
    if let Some(profile) = COLOR_PROFILE.get()
        && let Some(adapted) = profile.adapt_color(c)
    {
        c = adapted;
    }
    c
}
