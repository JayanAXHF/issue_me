use std::borrow::Cow;

use ratatui::{
    buffer::Buffer,
    layout::{Position, Rect},
    style::{Modifier, Style},
    widgets::Widget,
};
use textwrap::core::display_width;

#[derive(Debug, Clone)]
pub struct Link<'a> {
    label: Cow<'a, str>,
    url: Cow<'a, str>,
    style: Style,
    hover_style: Option<Style>,
    fallback_suffix: Option<Cow<'a, str>>,
    enabled: bool,
    focused: bool,
}

impl<'a> Link<'a> {
    pub fn new<L, U>(label: L, url: U) -> Self
    where
        L: Into<Cow<'a, str>>,
        U: Into<Cow<'a, str>>,
    {
        Self {
            label: label.into(),
            url: url.into(),
            style: Style::default()
                .fg(ratatui::style::Color::Blue)
                .add_modifier(Modifier::UNDERLINED),
            hover_style: None,
            fallback_suffix: None,
            enabled: true,
            focused: false,
        }
    }

    pub fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    pub fn hover_style(mut self, style: Style) -> Self {
        self.hover_style = Some(style);
        self
    }

    pub fn fallback_suffix<S>(mut self, suffix: S) -> Self
    where
        S: Into<Cow<'a, str>>,
    {
        self.fallback_suffix = Some(suffix.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    fn resolved_style(&self) -> Style {
        if self.focused {
            if let Some(hover_style) = self.hover_style {
                return self.style.patch(hover_style);
            }
            return self
                .style
                .patch(Style::default().add_modifier(Modifier::BOLD));
        }
        self.style
    }

    fn should_render_hyperlink(&self) -> bool {
        self.enabled && osc8_supported() && !self.url.is_empty()
    }
}

impl Widget for Link<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let style = self.resolved_style();
        let max_width = area.width as usize;
        let base_label = if self.label.is_empty() {
            self.url.as_ref()
        } else {
            self.label.as_ref()
        };

        let (mut label, was_truncated) = truncate_label(base_label, max_width);
        if !self.should_render_hyperlink()
            && was_truncated
            && let Some(suffix) = self.fallback_suffix.as_deref()
        {
            label = with_fallback_suffix(&label, suffix, max_width);
        }

        let label_width = display_width(&label).min(max_width);
        if label_width == 0 {
            return;
        }

        clear_area_row(area, buf);

        if self.should_render_hyperlink() {
            let encoded = encode_osc8(&label, &self.url);
            if let Some(first_cell) = buf.cell_mut(Position::new(area.x, area.y)) {
                first_cell.set_symbol(&encoded);
                first_cell.set_style(style);
                first_cell.set_skip(false);
            }

            for offset in 1..label_width {
                let x = area.x + offset as u16;
                if let Some(cell) = buf.cell_mut(Position::new(x, area.y)) {
                    cell.set_symbol(" ");
                    cell.set_skip(true);
                    cell.set_style(style);
                }
            }
            return;
        }

        buf.set_stringn(area.x, area.y, &label, max_width, style);
        for offset in 0..label_width {
            let x = area.x + offset as u16;
            if let Some(cell) = buf.cell_mut(Position::new(x, area.y)) {
                cell.set_skip(false);
            }
        }
    }
}

fn clear_area_row(area: Rect, buf: &mut Buffer) {
    for offset in 0..area.width {
        let x = area.x + offset;
        if let Some(cell) = buf.cell_mut(Position::new(x, area.y)) {
            cell.set_symbol(" ");
            cell.set_skip(false);
        }
    }
}

fn with_fallback_suffix(label: &str, suffix: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let suffix_width = display_width(suffix);
    if suffix_width >= max_width {
        let (truncated, _) = truncate_label(suffix, max_width);
        return truncated;
    }

    let prefix_max = max_width - suffix_width;
    let (prefix, _) = truncate_label(label, prefix_max);
    format!("{prefix}{suffix}")
}

fn truncate_label(label: &str, max_width: usize) -> (String, bool) {
    if max_width == 0 {
        return (String::new(), !label.is_empty());
    }
    if display_width(label) <= max_width {
        return (label.to_string(), false);
    }
    if max_width <= 3 {
        return (".".repeat(max_width), true);
    }

    let mut out = String::new();
    let mut width = 0usize;
    for ch in label.chars() {
        let mut char_buf = [0u8; 4];
        let ch_str = ch.encode_utf8(&mut char_buf);
        let ch_width = display_width(ch_str);
        if width + ch_width + 3 > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out.push_str("...");
    (out, true)
}

fn encode_osc8(label: &str, url: &str) -> String {
    format!("\u{1b}]8;;{url}\u{1b}\\{label}\u{1b}]8;;\u{1b}\\")
}

fn osc8_supported() -> bool {
    if let Ok(term) = std::env::var("TERM")
        && term == "dumb"
    {
        return false;
    }

    if let Ok(value) = std::env::var("ISSUE_ME_DISABLE_HYPERLINKS")
        && is_truthy(&value)
    {
        return false;
    }

    true
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn renders_hyperlink_and_marks_skip_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        Link::new("ratatui", "https://github.com/ratatui/").render(Rect::new(0, 0, 7, 1), &mut buf);

        let first = buf.cell(Position::new(0, 0)).expect("first cell");
        assert!(
            first
                .symbol()
                .contains("\u{1b}]8;;https://github.com/ratatui/\u{1b}\\")
        );

        for x in 1..7 {
            let cell = buf.cell(Position::new(x, 0)).expect("linked cell");
            assert!(cell.skip);
        }
    }

    #[test]
    fn clips_link_label_to_render_area() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        Link::new("ratatui", "https://github.com/ratatui/").render(Rect::new(0, 0, 4, 1), &mut buf);

        let first = buf.cell(Position::new(0, 0)).expect("first cell");
        assert!(first.symbol().contains("r..."));

        for x in 1..4 {
            let cell = buf.cell(Position::new(x, 0)).expect("clipped cell");
            assert!(cell.skip);
            assert_eq!(cell.symbol(), " ");
        }
    }

    #[test]
    fn clears_stale_skip_flags_between_renders() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));

        Link::new("long-link", "https://example.com").render(Rect::new(0, 0, 9, 1), &mut buf);
        Link::new("tiny", "https://example.com").render(Rect::new(0, 0, 9, 1), &mut buf);

        for x in 4..9 {
            let cell = buf.cell(Position::new(x, 0)).expect("tail cell");
            assert!(!cell.skip);
            assert_eq!(cell.symbol(), " ");
        }
    }

    #[test]
    fn falls_back_to_plain_text_when_disabled() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 1));
        Link::new("ratatui", "https://github.com/ratatui/")
            .enabled(false)
            .render(Rect::new(0, 0, 7, 1), &mut buf);

        assert_eq!(buf.cell(Position::new(0, 0)).expect("cell").symbol(), "r");
        assert_eq!(buf.cell(Position::new(1, 0)).expect("cell").symbol(), "a");
        assert!(!buf.cell(Position::new(1, 0)).expect("cell").skip);
    }

    #[test]
    fn no_op_for_zero_width_area() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 2, 1));
        Link::new("ratatui", "https://github.com/ratatui/").render(Rect::new(0, 0, 0, 1), &mut buf);

        assert_eq!(buf.cell(Position::new(0, 0)).expect("cell").symbol(), " ");
        assert_eq!(buf.cell(Position::new(1, 0)).expect("cell").symbol(), " ");
    }
}
