use std::sync::Arc;

mod support;

use crate::support::buffer_to_string;
use gitv_tui::ui::AppState;
use gitv_tui::ui::components::issue_detail::{IssuePreview, IssuePreviewSeed};
use gitv_tui::ui::layout::Layout;
use insta::assert_snapshot;
use octocrab::models::IssueState;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn render_issue_preview(seed: Option<IssuePreviewSeed>) -> String {
    let area = Rect::new(0, 0, 40, 20);
    let layout = Layout::new(area);
    let mut buf = Buffer::empty(area);

    let mut preview = IssuePreview::new(AppState::new(
        "owner".to_string(),
        "repo".to_string(),
        "user".to_string(),
    ));

    if let Some(s) = seed {
        preview.current = Some(s);
    }

    preview.render(layout, &mut buf);
    buffer_to_string(&buf)
}

#[test]
fn issue_preview_open_issue() {
    let seed = IssuePreviewSeed {
        number: 42,
        state: IssueState::Open,
        author: Arc::from("johndoe"),
        created_at: Arc::from("2024-01-15 10:30"),
        updated_at: Arc::from("2024-01-16 14:45"),
        comments: 5,
        assignees: vec![Arc::from("alice"), Arc::from("bob")],
        milestone: Some(Arc::from("v1.0")),
        is_pull_request: false,
        pull_request_url: None,
    };
    let result = render_issue_preview(Some(seed));
    assert_snapshot!(result);
}

#[test]
fn issue_preview_closed_issue() {
    let seed = IssuePreviewSeed {
        number: 123,
        state: IssueState::Closed,
        author: Arc::from("janedoe"),
        created_at: Arc::from("2023-12-01 09:00"),
        updated_at: Arc::from("2023-12-05 16:30"),
        comments: 12,
        assignees: vec![Arc::from("charlie")],
        milestone: None,
        is_pull_request: false,
        pull_request_url: None,
    };
    let result = render_issue_preview(Some(seed));
    assert_snapshot!(result);
}

#[test]
fn issue_preview_pull_request() {
    let seed = IssuePreviewSeed {
        number: 456,
        state: IssueState::Open,
        author: Arc::from("devuser"),
        created_at: Arc::from("2024-02-01 11:00"),
        updated_at: Arc::from("2024-02-02 09:15"),
        comments: 8,
        assignees: vec![Arc::from("reviewer1"), Arc::from("reviewer2")],
        milestone: Some(Arc::from("Sprint 5")),
        is_pull_request: true,
        pull_request_url: Some(Arc::from("https://github.com/owner/repo/pull/456")),
    };
    let result = render_issue_preview(Some(seed));
    assert_snapshot!(result);
}

#[test]
fn issue_preview_no_selection() {
    let result = render_issue_preview(None);
    assert_snapshot!(result);
}

#[test]
fn issue_preview_many_assignees() {
    let seed = IssuePreviewSeed {
        number: 789,
        state: IssueState::Open,
        author: Arc::from("teamlead"),
        created_at: Arc::from("2024-03-01 08:00"),
        updated_at: Arc::from("2024-03-02 10:00"),
        comments: 3,
        assignees: vec![
            Arc::from("dev1"),
            Arc::from("dev2"),
            Arc::from("dev3"),
            Arc::from("dev4"),
            Arc::from("dev5"),
        ],
        milestone: Some(Arc::from("v2.0")),
        is_pull_request: false,
        pull_request_url: None,
    };
    let result = render_issue_preview(Some(seed));
    assert_snapshot!(result);
}
