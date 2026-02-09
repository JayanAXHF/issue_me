use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;
use octocrab::models::{Event, IssueState};
use rat_widget::focus::{FocusBuilder, FocusFlag, HasFocus, Navigation};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    prelude::Widget,
    style::Style,
    text::{Line, Span, Text},
    widgets::{Block, Paragraph, StatefulWidget, Wrap},
};
use ratatui_macros::line;
use throbber_widgets_tui::{BRAILLE_SIX_DOUBLE, Throbber, ThrobberState, WhichUse};

use crate::{
    app::GITHUB_CLIENT,
    ui::{Action, AppState, components::Component, layout::Layout, utils::get_border_style},
};

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
    action_tx: Option<tokio::sync::mpsc::Sender<Action>>,
    current: Option<IssuePreviewSeed>,
    cache: HashMap<u64, Vec<PrSummary>>,
    loading: HashSet<u64>,
    error: Option<String>,
    owner: String,
    repo: String,
    focus: FocusFlag,
    area: Rect,
    throbber_state: ThrobberState,
}

impl IssuePreview {
    pub fn new(AppState { repo, owner, .. }: AppState) -> Self {
        Self {
            action_tx: None,
            current: None,
            cache: HashMap::new(),
            loading: HashSet::new(),
            error: None,
            owner,
            repo,
            focus: FocusFlag::new().with_name("issue_preview"),
            area: Rect::default(),
            throbber_state: ThrobberState::default(),
        }
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.area = area.issue_preview;
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(self))
            .title("Issue Info");

        let (text, loading) = self.build_text();
        let inner = block.inner(area.issue_preview);
        let widget = Paragraph::new(text).block(block).wrap(Wrap { trim: true });
        widget.render(area.issue_preview, buf);

        if let Some(loading) = loading {
            let x = inner.x.saturating_add(loading.col);
            let y = inner.y.saturating_add(loading.line);
            if y < inner.y.saturating_add(inner.height) && x < inner.x.saturating_add(inner.width) {
                let width = inner.width.saturating_sub(loading.col).clamp(1, 8);
                let area = Rect {
                    x,
                    y,
                    width,
                    height: 1,
                };
                let throbber = Throbber::default()
                    .style(Style::new().cyan())
                    .throbber_set(BRAILLE_SIX_DOUBLE)
                    .use_type(WhichUse::Spin);
                StatefulWidget::render(throbber, area, buf, &mut self.throbber_state);
            }
        }
    }

    fn build_text(&self) -> (Text<'_>, Option<LoadingIndicator>) {
        let mut lines: Vec<Line<'_>> = Vec::new();
        let label_style = Style::new().dim();
        let mut loading = None;

        let Some(seed) = &self.current else {
            lines.push(line![Span::styled(
                "Select an issue to see details.",
                Style::new().dim()
            )]);
            return (Text::from(lines), None);
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

        let open_prs = self.cache.get(&seed.number);
        if self.loading.contains(&seed.number) {
            let label = "Open PRs: ";
            loading = Some(LoadingIndicator {
                line: lines.len() as u16,
                col: label.len() as u16,
            });
            lines.push(Line::from(vec![Span::styled(label, label_style)]));
        } else if let Some(err) = &self.error {
            lines.push(Line::from(vec![
                Span::styled("Open PRs: ", label_style),
                Span::styled(err.clone(), Style::new().red()),
            ]));
        } else if let Some(prs) = open_prs {
            if prs.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("Open PRs: ", label_style),
                    Span::styled("None", Style::new().dim()),
                ]));
            } else {
                lines.push(Line::from(vec![Span::styled("Open PRs:", label_style)]));
                for pr in prs.iter().take(3) {
                    let pr_state = match pr.state {
                        IssueState::Open => Style::new().green(),
                        IssueState::Closed => Style::new().magenta(),
                        _ => Style::new().cyan(),
                    };
                    lines.push(Line::from(vec![
                        Span::raw("  #"),
                        Span::styled(pr.number.to_string(), Style::new().yellow()),
                        Span::raw(" "),
                        Span::styled(pr.title.as_ref(), pr_state),
                    ]));
                }
                if prs.len() > 3 {
                    let more = prs.len() - 3;
                    lines.push(Line::from(vec![
                        Span::raw("  +"),
                        Span::styled(more.to_string(), Style::new().dim()),
                        Span::styled(" more", Style::new().dim()),
                    ]));
                }
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled("Open PRs: ", label_style),
                Span::styled("None", Style::new().dim()),
            ]));
        }

        (Text::from(lines), loading)
    }

    async fn fetch_open_prs(&mut self, issue_number: u64) {
        if self.loading.contains(&issue_number) {
            return;
        }
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.loading.insert(issue_number);
        self.error = None;

        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::IssuePreviewError {
                        number: issue_number,
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };

            let handler = client.inner().issues(owner, repo);
            let page = handler
                .list_timeline_events(issue_number)
                .per_page(100u8)
                .page(1u32)
                .send()
                .await;

            match page {
                Ok(mut p) => {
                    let mut seen = HashSet::new();
                    let mut prs = Vec::new();
                    for event in std::mem::take(&mut p.items) {
                        if matches!(event.event, Event::Connected | Event::CrossReferenced)
                            && let Some(source) = event.source
                        {
                            let src_issue = source.issue;
                            if src_issue.pull_request.is_some()
                                && matches!(src_issue.state, IssueState::Open)
                                && seen.insert(src_issue.number)
                            {
                                prs.push(PrSummary {
                                    number: src_issue.number,
                                    title: Arc::<str>::from(src_issue.title),
                                    state: src_issue.state,
                                });
                            }
                        }
                    }
                    let _ = action_tx
                        .send(Action::IssuePreviewLoaded {
                            number: issue_number,
                            open_prs: prs,
                        })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::IssuePreviewError {
                            number: issue_number,
                            message: err.to_string().replace('\n', " "),
                        })
                        .await;
                }
            }
        });
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
        match event {
            Action::SelectedIssuePreview { seed } => {
                let number = seed.number;
                self.current = Some(seed);
                if self.cache.contains_key(&number) {
                    self.loading.remove(&number);
                    self.error = None;
                } else {
                    self.fetch_open_prs(number).await;
                }
            }
            Action::IssuePreviewLoaded { number, open_prs } => {
                self.cache.insert(number, open_prs);
                self.loading.remove(&number);
                if self.current.as_ref().is_some_and(|s| s.number == number) {
                    self.error = None;
                }
            }
            Action::IssuePreviewError { number, message } => {
                self.loading.remove(&number);
                if self.current.as_ref().is_some_and(|s| s.number == number) {
                    self.error = Some(message);
                }
            }
            Action::Tick => {
                if self
                    .current
                    .as_ref()
                    .is_some_and(|s| self.loading.contains(&s.number))
                {
                    self.throbber_state.calc_next();
                }
            }
            _ => {}
        }
    }

    fn is_animating(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|s| self.loading.contains(&s.number))
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

struct LoadingIndicator {
    line: u16,
    col: u16,
}
