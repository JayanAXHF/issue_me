use std::sync::Arc;

use async_trait::async_trait;
use octocrab::models::IssueState;
use rat_widget::focus::{FocusBuilder, FocusFlag, HasFocus, Navigation};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    prelude::Widget,
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, Wrap},
};
use ratatui_macros::line;

use crate::ui::{
    Action, AppState,
    components::{Component, help::HelpElementKind},
    layout::Layout,
    utils::get_border_style,
};

pub const HELP: &[HelpElementKind] = &[
    crate::help_text!("Issue Preview Help"),
    crate::help_text!("Read-only panel showing metadata for the selected issue."),
    crate::help_keybind!("1-6", "use global focus keys to switch components"),
];

#[derive(Debug, Clone)]
pub struct IssuePreviewSeed {
    pub number: u64,
    pub state: IssueState,
    pub author: Arc<str>,
    pub created_at: Arc<str>,
    pub updated_at: Arc<str>,
    pub comments: u32,
    pub assignees: Vec<Arc<str>>,
    pub milestone: Option<Arc<str>>,
    pub is_pull_request: bool,
    pub pull_request_url: Option<Arc<str>>,
}

impl IssuePreviewSeed {
    pub fn from_issue(issue: &octocrab::models::issues::Issue) -> Self {
        let assignees = issue
            .assignees
            .iter()
            .map(|a| Arc::<str>::from(a.login.as_str()))
            .collect();
        let milestone = issue
            .milestone
            .as_ref()
            .map(|m| Arc::<str>::from(m.title.as_str()));
        Self {
            number: issue.number,
            state: issue.state.clone(),
            author: Arc::<str>::from(issue.user.login.as_str()),
            created_at: Arc::<str>::from(issue.created_at.format("%Y-%m-%d %H:%M").to_string()),
            updated_at: Arc::<str>::from(issue.updated_at.format("%Y-%m-%d %H:%M").to_string()),
            comments: issue.comments,
            assignees,
            milestone,
            is_pull_request: issue.pull_request.is_some(),
            pull_request_url: issue
                .pull_request
                .as_ref()
                .map(|pr| Arc::<str>::from(pr.html_url.as_str())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrSummary {
    pub number: u64,
    pub title: Arc<str>,
    pub state: IssueState,
}

pub struct IssuePreview {
    current: Option<IssuePreviewSeed>,
    action_tx: Option<tokio::sync::mpsc::Sender<Action>>,
    focus: FocusFlag,
    area: Rect,
}

impl IssuePreview {
    pub fn new(_: AppState) -> Self {
        Self {
            current: None,
            action_tx: None,
            focus: FocusFlag::new().with_name("issue_preview"),
            area: Rect::default(),
        }
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.area = area.issue_preview;
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(self))
            .title("Issue Info");

        let text = self.build_text();
        let widget = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        widget.render(area.issue_preview, buf);
    }

    fn build_text(&self) -> Text<'_> {
        let mut lines: Vec<Line<'_>> = Vec::new();
        let label_style = Style::new().dim();

        let Some(seed) = &self.current else {
            lines.push(line![Span::styled(
                "Select an issue to see details.",
                Style::new().dim()
            )]);
            return Text::from(lines);
        };

        let state_style = match seed.state {
            IssueState::Open => Style::new().green(),
            IssueState::Closed => Style::new().magenta(),
            _ => Style::new().cyan(),
        };

        let kind = if seed.is_pull_request {
            "Pull Request"
        } else {
            "Issue"
        };
        lines.push(Line::from(vec![
            Span::styled("Type: ", label_style),
            Span::styled(kind, Style::new().cyan()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("State: ", label_style),
            Span::styled(format!("{:?}", seed.state), state_style),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Author: ", label_style),
            Span::styled(seed.author.as_ref(), Style::new().cyan()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Created: ", label_style),
            Span::styled(seed.created_at.as_ref(), Style::new().dim()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Updated: ", label_style),
            Span::styled(seed.updated_at.as_ref(), Style::new().dim()),
        ]));
        lines.push(Line::from(vec![
            Span::styled("Comments: ", label_style),
            Span::styled(seed.comments.to_string(), Style::new().yellow()),
        ]));

        let assignees = summarize_list(&seed.assignees, 3);
        lines.push(Line::from(vec![
            Span::styled("Assignees: ", label_style),
            Span::styled(assignees, Style::new().white()),
        ]));

        let milestone = seed
            .milestone
            .as_ref()
            .map(|m| m.as_ref())
            .unwrap_or("None");
        lines.push(Line::from(vec![
            Span::styled("Milestone: ", label_style),
            Span::styled(milestone, Style::new().light_blue()),
        ]));

        if seed.is_pull_request && matches!(seed.state, IssueState::Open) {
            lines.push(Line::from(vec![Span::styled("Open PRs:", label_style)]));
            lines.push(Line::from(vec![
                Span::raw("  #"),
                Span::styled(seed.number.to_string(), Style::new().yellow()),
                Span::raw(" "),
                Span::styled("(this issue is a PR)", Style::new().green()),
            ]));
            if let Some(pr_url) = &seed.pull_request_url {
                lines.push(Line::from(vec![
                    Span::styled("  url: ", label_style),
                    Span::styled(pr_url.as_ref(), Style::new().fg(Color::Blue)),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled("Open PRs: ", label_style),
                Span::styled("None", Style::new().dim()),
            ]));
        }

        Text::from(lines)
    }
}

#[async_trait(?Send)]
impl Component for IssuePreview {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }

    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        self.action_tx = Some(action_tx);
    }

    async fn handle_event(&mut self, event: Action) {
        if let Action::SelectedIssuePreview { seed } = event {
            self.current = Some(seed);
        }
    }

    fn set_global_help(&self) {
        if let Some(action_tx) = &self.action_tx {
            let _ = action_tx.try_send(Action::SetHelp(HELP));
        }
    }
}

impl HasFocus for IssuePreview {
    fn build(&self, builder: &mut FocusBuilder) {
        builder.leaf_widget(self);
    }

    fn focus(&self) -> FocusFlag {
        self.focus.clone()
    }

    fn area(&self) -> Rect {
        self.area
    }

    fn navigable(&self) -> Navigation {
        Navigation::None
    }
}

fn summarize_list(items: &[Arc<str>], max: usize) -> String {
    if items.is_empty() {
        return "None".to_string();
    }
    if items.len() <= max {
        return items
            .iter()
            .map(|s| s.as_ref())
            .collect::<Vec<_>>()
            .join(", ");
    }
    let shown = items
        .iter()
        .take(max)
        .map(|s| s.as_ref())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{shown} +{} more", items.len() - max)
}
