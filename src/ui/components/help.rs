use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{BlockExt, Clear, Widget},
};
use tracing::info;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpElementKind {
    Keybind(&'static str, &'static str),
    Text(&'static str),
}

#[macro_export]
macro_rules! help_keybind {
    ($key:expr, $description:expr) => {
        $crate::ui::components::help::HelpElementKind::Keybind($key, $description)
    };
}

#[macro_export]
macro_rules! help_text {
    ($text:expr) => {
        $crate::ui::components::help::HelpElementKind::Text($text)
    };
}

pub fn help_elements_to_text(elements: &[HelpElementKind], width: u16) -> Text<'static> {
    let mut lines = Vec::with_capacity(elements.len());
    for element in elements {
        match element {
            HelpElementKind::Keybind(key, description) => {
                let total_length = (key.len() + description.len()) as u16; // +1 for the space between
                let padding = if total_length < width {
                    width - total_length
                } else {
                    1 // Ensure at least one space if the content exceeds the width
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        *key,
                        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" ".repeat(padding as usize)),
                    Span::raw(*description),
                ]));
            }
            HelpElementKind::Text(text) => {
                let wrapped = textwrap::wrap(text, width as usize);
                lines.extend(wrapped.into_iter().map(|line| Line::from(line).centered()));
            }
        }
    }
    Text::from(lines)
}

/// A simple component to display help information. It can be centered within its parent area using the `set_constraints` method.
pub struct HelpComponent<'a> {
    contraint: u16,
    content: &'a [HelpElementKind],
    block: Option<ratatui::widgets::Block<'a>>,
    width: u16,
}

impl<'a> HelpComponent<'a> {
    /// Creates a new HelpComponent with the given content.
    pub fn new(content: &'a [HelpElementKind]) -> Self {
        Self {
            content,
            width: 0,
            contraint: 0,
            block: None,
        }
    }
    /// Sets the constraints for centering the component. The constraints are specified as percentages of the parent area.
    pub fn set_constraint(self, contraint: u16) -> Self {
        Self { contraint, ..self }
    }
    /// Sets a block around the component. This can be used to visually separate the help content from other UI elements.
    pub fn block(self, block: ratatui::widgets::Block<'a>) -> Self {
        Self {
            block: Some(block),
            ..self
        }
    }
}

impl<'a> Widget for HelpComponent<'a> {
    fn render(mut self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        use ratatui::layout::Constraint::{Length, Percentage};
        info!(content = ?self.content, "Rendering HelpComponent");
        info!(content_length = ?self.content.len(), "Content length");
        let mut centered_area = if self.contraint != 0 {
            area.centered(Percentage(self.contraint), Length(self.contraint))
        } else {
            area
        };
        let mut inner = self.block.inner_if_some(centered_area);
        self.width = inner.width;
        let text = help_elements_to_text(self.content, self.width);
        let text_height = text.height() as u16;
        let y_offset = |h: u16| {
            if text_height < h {
                (h - text_height) / 2
            } else {
                0
            }
        };
        inner.y += y_offset(inner.height) + 1;
        inner.height = text.height() as u16;
        let inner_height = inner.height;
        centered_area.y += y_offset(centered_area.height);
        centered_area.height = inner_height + 2;
        Clear.render(centered_area, buf);
        self.block.render(centered_area, buf);
        text.render(inner, buf);
    }
}
