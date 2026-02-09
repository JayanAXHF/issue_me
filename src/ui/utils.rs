use rat_widget::focus::HasFocus;
use ratatui::{layout::Rect, style::Style};

pub fn get_loader_area(area: Rect) -> Rect {
    Rect {
        x: area.width - 10,
        y: area.y,
        width: 10,
        height: 1,
    }
}

#[inline(always)]
pub fn get_border_style(state: &impl HasFocus) -> Style {
    let default_border_style = Style::default();
    let focused_border_style = Style::default().yellow();
    if state.is_focused() {
        focused_border_style
    } else {
        default_border_style
    }
}
