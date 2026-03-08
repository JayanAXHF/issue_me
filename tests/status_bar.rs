mod support;

use crate::support::buffer_to_string;
use gitv_tui::ui::AppState;
use gitv_tui::ui::components::issue_list::LOADED_ISSUE_COUNT;
use gitv_tui::ui::components::status_bar::StatusBar;
use gitv_tui::ui::layout::Layout;
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::sync::atomic::Ordering;

fn render_status_bar(issue_count: u32) -> String {
    LOADED_ISSUE_COUNT.store(issue_count, Ordering::Relaxed);

    let area = Layout::new(Rect::new(0, 0, 80, 3));
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 3));

    let mut status_bar = StatusBar::new(AppState::new(
        "owner".to_string(),
        "repo".to_string(),
        "testuser".to_string(),
    ));

    status_bar.render(area, &mut buf);
    buffer_to_string(&buf)
}

#[test]
fn status_bar_with_count() {
    let result = render_status_bar(42);
    assert_snapshot!(result);
}
