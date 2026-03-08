mod support;
use crate::support::buffer_to_string;
use gitv_tui::ui::components::help::{HelpComponent, HelpElementKind};
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Widget};

fn render_help_component(elements: &[HelpElementKind], width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    let component = HelpComponent::new(elements).set_constraint(50).block(
        Block::bordered()
            .title("Help")
            .padding(ratatui::widgets::Padding::horizontal(2))
            .border_type(ratatui::widgets::BorderType::Rounded),
    );
    component.render(area, &mut buf);
    buffer_to_string(&buf)
}

#[test]
fn help_elements_keybinds_only() {
    let elements = &[
        HelpElementKind::Keybind("Up", "navigate up"),
        HelpElementKind::Keybind("Down", "navigate down"),
        HelpElementKind::Keybind("Enter", "select item"),
    ];
    let result = render_help_component(elements, 60, 20);
    assert_snapshot!(result);
}

#[test]
fn help_elements_text_wrapping() {
    let elements = &[HelpElementKind::Text(
        "This is a very long description that should wrap properly across multiple lines when rendered in the help component.",
    )];
    let result = render_help_component(elements, 50, 15);
    assert_snapshot!(result);
}

#[test]
fn help_elements_mixed_content() {
    let elements = &[
        HelpElementKind::Text("Global Help"),
        HelpElementKind::Keybind("q", "quit application"),
        HelpElementKind::Text(""),
        HelpElementKind::Keybind("?", "toggle this help"),
        HelpElementKind::Keybind("Esc", "close dialog"),
    ];
    let result = render_help_component(elements, 55, 20);
    assert_snapshot!(result);
}

#[test]
fn help_component_empty() {
    let elements: &[HelpElementKind] = &[];
    let result = render_help_component(elements, 40, 10);
    assert_snapshot!(result);
}
