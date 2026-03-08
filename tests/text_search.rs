mod support;
use crate::support::buffer_to_string;
use gitv_tui::ui::AppState;
use gitv_tui::ui::components::Component;
use gitv_tui::ui::components::search_bar::TextSearch;
use gitv_tui::ui::layout::Layout;
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn render_text_search<F>(setup: F) -> String
where
    F: FnOnce(&mut TextSearch),
{
    let area = Layout::new(Rect::new(0, 0, 80, 10));
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 10));

    let mut search = TextSearch::new(AppState::new(
        "owner".to_string(),
        "repo".to_string(),
        "user".to_string(),
    ));

    setup(&mut search);

    search.render(area, &mut buf);
    buffer_to_string(&buf)
}

#[test]
fn text_search_loaded_state() {
    let result = render_text_search(|_| {});
    assert_snapshot!(result);
}

#[test]
fn text_search_with_input() {
    let result = render_text_search(|search| {
        search.search_state.set_text("bug fix");
    });
    assert_snapshot!(result);
}

#[test]
fn text_search_label_input() {
    let result = render_text_search(|search| {
        search.label_state.set_text("priority:high");
    });
    assert_snapshot!(result);
}

#[test]
fn text_search_both_inputs() {
    let result = render_text_search(|search| {
        search.search_state.set_text("authentication");
        search.label_state.set_text("security;bug");
    });
    assert_snapshot!(result);
}
