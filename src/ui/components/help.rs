use ratatui::widgets::{BlockExt, Clear, Paragraph, Widget};

/// A simple component to display help information. It can be centered within its parent area using the `set_constraints` method.
pub struct HelpComponent<'a> {
    contraints: [u16; 2],
    content: Paragraph<'a>,
    block: Option<ratatui::widgets::Block<'a>>,
}

impl<'a> HelpComponent<'a> {
    /// Creates a new HelpComponent with the given content.
    pub fn new(content: Paragraph<'a>) -> Self {
        Self {
            content,
            contraints: [0, 0],
            block: None,
        }
    }
    /// Sets the constraints for centering the component. The constraints are specified as percentages of the parent area.
    pub fn set_constraints(self, contraints: [u16; 2]) -> Self {
        Self { contraints, ..self }
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
    fn render(self, area: ratatui::layout::Rect, buf: &mut ratatui::buffer::Buffer) {
        use ratatui::layout::Constraint::Percentage;
        let centered_area = if self.contraints != [0, 0] {
            area.centered(
                Percentage(self.contraints[0]),
                Percentage(self.contraints[1]),
            )
        } else {
            area
        };
        let inner = self.block.inner_if_some(centered_area);
        Clear.render(centered_area, buf);
        self.block.render(centered_area, buf);
        self.content.render(inner, buf);
    }
}
