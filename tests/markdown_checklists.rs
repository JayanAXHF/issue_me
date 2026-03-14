mod support;

use crate::support::buffer_to_string;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use futures::executor::block_on;
use gitv_tui::ui::components::{Component, issue_create::IssueCreate};
use gitv_tui::ui::issue_data::UiIssuePool;
use gitv_tui::ui::{Action, AppState};
use insta::assert_snapshot;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

fn render_issue_create_preview(body: &str, width: u16, height: u16) -> String {
    let area = Rect::new(0, 0, width, height);
    let layout = gitv_tui::ui::layout::Layout::fullscreen(area);
    let mut buf = Buffer::empty(area);
    let issue_pool = Arc::new(RwLock::new(UiIssuePool::default()));
    let mut issue_create = IssueCreate::new(
        AppState::new("repo".to_string(), "owner".to_string(), "user".to_string()),
        issue_pool,
    );
    let (tx, _rx) = mpsc::channel(8);
    issue_create.register_action_tx(tx);

    block_on(async {
        issue_create
            .handle_event(Action::EnterIssueCreate)
            .await
            .expect("enter issue create should succeed");
        issue_create
            .handle_event(Action::AppEvent(Event::Paste(body.to_string())))
            .await
            .expect("pasting markdown body should succeed");
        issue_create
            .handle_event(Action::AppEvent(Event::Key(KeyEvent::new(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL,
            ))))
            .await
            .expect("toggling preview should succeed");
    });

    issue_create.render(layout, &mut buf);
    buffer_to_string(&buf)
}

#[test]
fn markdown_preview_renders_ascii_checklists() {
    let result = render_issue_create_preview(
        "## Tasks\n\n- [ ] write docs\n- [x] add tests\n- plain bullet",
        44,
        16,
    );

    assert_snapshot!(result);
}

#[test]
fn markdown_preview_wraps_checklist_continuations() {
    let result = render_issue_create_preview(
        "- [ ] a very long checklist item that should wrap onto the next rendered line",
        34,
        14,
    );

    assert_snapshot!(result);
}

#[test]
fn markdown_preview_keeps_checked_and_unchecked_prefixes() {
    let result = render_issue_create_preview(
        "- [x] completed task\n- [ ] pending task\n- [x] reviewed",
        38,
        14,
    );

    assert_snapshot!(result);
}
