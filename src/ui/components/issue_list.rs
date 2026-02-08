use async_trait::async_trait;
use octocrab::{
    Page,
    issues::IssueHandler,
    models::{IssueState, issues::Issue},
};
use rat_widget::{event::HandleEvent, focus::HasFocus, list::selection::RowSelection};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style, Stylize},
    symbols,
    widgets::{Block, ListItem, Padding, StatefulWidget},
};
use ratatui_macros::{line, span};
use textwrap::{Options, wrap};
use throbber_widgets_tui::ThrobberState;

use crate::{
    app::GITHUB_CLIENT,
    errors::AppError,
    ui::{Action, components::Component, layout::Layout},
};

pub struct IssueList<'a> {
    pub issues: Vec<IssueListItem>,
    pub page: Option<Page<Issue>>,
    pub list_state: rat_widget::list::ListState<RowSelection>,
    pub handler: IssueHandler<'a>,
    pub action_tx: Option<tokio::sync::mpsc::Sender<crate::ui::Action>>,
    pub throbber_state: ThrobberState,
    state: State,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum State {
    #[default]
    Loading,
    Loaded,
}

impl<'a> IssueList<'a> {
    pub async fn new(
        handler: IssueHandler<'a>,
        owner: String,
        repo: String,
        tx: tokio::sync::mpsc::Sender<Action>,
    ) -> Self {
        tokio::spawn(async move {
            let p = GITHUB_CLIENT
                .get()
                .unwrap()
                .inner()
                .issues(owner, repo)
                .list()
                .page(1_u32)
                .per_page(10u8)
                .send()
                .await
                .unwrap();
            tx.send(Action::NewPage(Box::new(p))).await.unwrap();
        });
        Self {
            page: None,
            throbber_state: ThrobberState::default(),
            action_tx: None,
            issues: vec![],
            list_state: rat_widget::list::ListState::default(),
            handler,
            state: State::default(),
        }
    }
    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .padding(Padding::horizontal(3));
        let list = rat_widget::list::List::<RowSelection>::new(
            self.issues.iter().map(Into::<ListItem>::into),
        )
        .block(block)
        .style(Style::default())
        .select_style(Style::default().add_modifier(Modifier::BOLD));
        list.render(area.main_content, buf, &mut self.list_state);
        if self.state == State::Loading {
            let title_area = Rect {
                x: area.main_content.x + 1,
                y: area.main_content.y,
                width: 10,
                height: 1,
            };
            let full = throbber_widgets_tui::Throbber::default()
                .label("Loading")
                .style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan))
                .throbber_set(throbber_widgets_tui::BRAILLE_SIX_DOUBLE)
                .use_type(throbber_widgets_tui::WhichUse::Spin);
            full.render(title_area, buf, &mut self.throbber_state);
        }
    }
}

pub struct IssueListItem(pub Issue);

impl std::ops::Deref for IssueListItem {
    type Target = Issue;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Issue> for IssueListItem {
    fn from(issue: Issue) -> Self {
        Self(issue)
    }
}

impl From<&IssueListItem> for ListItem<'_> {
    fn from(value: &IssueListItem) -> Self {
        let options = Options::with_termwidth();
        let binding = value.body.clone().unwrap_or("No desc provided".to_string());
        let mut body = wrap(binding.trim(), options);
        body.truncate(2);

        let lines = vec![
            line![
                "   ",
                span!(value.0.title.as_str()),
                " ",
                span!("#{}", value.0.number).dim(),
            ],
            line![
                span!(symbols::shade::FULL).style({
                    if matches!(value.0.state, IssueState::Open) {
                        Style::new().green()
                    } else {
                        Style::new().magenta()
                    }
                }),
                "  ",
                span!(
                    "Opened by {} at {}",
                    value.0.user.login,
                    value.0.created_at.format("%Y-%m-%d %H:%M:%S")
                )
                .dim(),
            ],
            line!["   ", span!(body.join(" ")).style(Style::new().dim())],
        ];
        ListItem::new(lines)
    }
}

#[async_trait(?Send)]
impl Component for IssueList<'_> {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }

    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<crate::ui::Action>) {
        self.action_tx = Some(action_tx);
    }
    async fn handle_event(&mut self, event: crate::ui::Action) {
        match event {
            crate::ui::Action::Tick => {
                if self.state == State::Loading {
                    self.throbber_state.calc_next();
                }
            }
            crate::ui::Action::AppEvent(ref event) => {
                if let rat_widget::event::Outcome::Changed =
                    self.list_state.handle(event, rat_widget::event::Regular)
                {
                    let selected = self.list_state.selected_checked();
                    if let Some(selected) = selected {
                        if selected == self.issues.len() - 1
                            && let Some(page) = &self.page
                        {
                            let tx = self.action_tx.as_ref().unwrap().clone();
                            let page_next = page.next.clone();
                            self.state = State::Loading;
                            tokio::spawn(async move {
                                let p = GITHUB_CLIENT
                                    .get()
                                    .unwrap()
                                    .inner()
                                    .get_page::<Issue>(&page_next)
                                    .await;
                                if let Ok(pres) = p
                                    && let Some(p) = pres
                                {
                                    tx.send(crate::ui::Action::NewPage(Box::new(p))).await?;
                                }
                                Ok::<(), AppError>(())
                            });
                        }
                        let labels = &self.issues[selected].0.labels;
                        self.action_tx
                            .as_ref()
                            .unwrap()
                            .send(crate::ui::Action::ChangeLabels(labels.clone()))
                            .await
                            .unwrap();
                    }
                }
            }
            crate::ui::Action::NewPage(mut p) => {
                let issues = std::mem::take(&mut p.items)
                    .into_iter()
                    .map(IssueListItem::from);
                let prev_issues = std::mem::take(&mut self.issues);
                self.issues = prev_issues.into_iter().chain(issues).collect();
                self.page = Some(*p);
                self.state = State::Loaded;
            }
            _ => {}
        }
    }
}

impl HasFocus for IssueList<'_> {
    fn build(&self, builder: &mut rat_widget::focus::FocusBuilder) {
        let tag = builder.start(self);
        builder.widget(&self.list_state);
        builder.end(tag);
    }
    fn area(&self) -> ratatui::layout::Rect {
        self.list_state.area()
    }
    fn focus(&self) -> rat_widget::focus::FocusFlag {
        self.list_state.focus()
    }
}
