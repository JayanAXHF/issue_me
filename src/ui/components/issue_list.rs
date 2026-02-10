use crate::{
    app::GITHUB_CLIENT,
    errors::AppError,
    ui::{
        Action,
        components::{
            Component, issue_conversation::IssueConversationSeed, issue_detail::IssuePreviewSeed,
        },
        layout::Layout,
        utils::get_border_style,
    },
};
use async_trait::async_trait;
use octocrab::{
    Page,
    issues::IssueHandler,
    models::{IssueState, issues::Issue},
};
use rat_widget::{
    event::{HandleEvent, ct_event},
    focus::{HasFocus, Navigation},
    list::selection::RowSelection,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style, Stylize},
    symbols,
    widgets::{Block, ListItem, Padding, StatefulWidget},
};
use ratatui_macros::{line, span};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use textwrap::{Options, wrap};
use throbber_widgets_tui::ThrobberState;
use tracing::info;

pub static LOADED_ISSUE_COUNT: AtomicU32 = AtomicU32::new(0);

pub struct IssueList<'a> {
    pub issues: Vec<IssueListItem>,
    pub page: Option<Arc<Page<Issue>>>,
    pub list_state: rat_widget::list::ListState<RowSelection>,
    pub handler: IssueHandler<'a>,
    pub action_tx: Option<tokio::sync::mpsc::Sender<crate::ui::Action>>,
    pub throbber_state: ThrobberState,
    state: State,
    pub screen: MainScreen,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum State {
    #[default]
    Loading,
    Loaded,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MainScreen {
    #[default]
    List,
    Details,
}

impl<'a> IssueList<'a> {
    pub async fn new(
        handler: IssueHandler<'a>,
        owner: String,
        repo: String,
        tx: tokio::sync::mpsc::Sender<Action>,
    ) -> Self {
        LOADED_ISSUE_COUNT.store(0, Ordering::Relaxed);
        tokio::spawn(async move {
            let Ok(mut p) = GITHUB_CLIENT
                .get()
                .unwrap()
                .inner()
                .issues(owner, repo)
                .list()
                .page(1_u32)
                .per_page(15u8)
                .send()
                .await
            else {
                return;
            };
            let items = std::mem::take(&mut p.items);
            let items = items
                .into_iter()
                .filter(|i| i.pull_request.is_none())
                .collect();
            p.items = items;

            tx.send(Action::NewPage(Arc::new(p))).await.unwrap();
        });
        Self {
            page: None,
            throbber_state: ThrobberState::default(),
            action_tx: None,
            issues: vec![],
            list_state: rat_widget::list::ListState::default(),
            handler,
            screen: MainScreen::default(),
            state: State::default(),
        }
    }
    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&self.list_state))
            .padding(Padding::horizontal(3));
        let list = rat_widget::list::List::<RowSelection>::new(
            self.issues.iter().map(Into::<ListItem>::into),
        )
        .block(block)
        .style(Style::default())
        .focus_style(Style::default().reversed().add_modifier(Modifier::BOLD));
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
                if self.screen != MainScreen::List {
                    return;
                }
                if matches!(event, ct_event!(keycode press Enter)) {
                    if let Some(selected) = self.list_state.selected_checked() {
                        let issue = &self.issues[selected].0;
                        self.action_tx
                            .as_ref()
                            .unwrap()
                            .send(crate::ui::Action::EnterIssueDetails {
                                seed: IssueConversationSeed::from_issue(issue),
                            })
                            .await
                            .unwrap();
                        self.action_tx
                            .as_ref()
                            .unwrap()
                            .send(crate::ui::Action::ChangeIssueScreen(MainScreen::Details))
                            .await
                            .unwrap();
                    }
                    return;
                }

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
                                    && let Some(mut p) = pres
                                {
                                    let items = std::mem::take(&mut p.items);
                                    let items = items
                                        .into_iter()
                                        .filter(|i| i.pull_request.is_none())
                                        .collect();
                                    p.items = items;
                                    tx.send(crate::ui::Action::NewPage(Arc::new(p))).await?;
                                }
                                tx.send(crate::ui::Action::FinishedLoading).await.unwrap();
                                Ok::<(), AppError>(())
                            });
                        }
                        let issue = &self.issues[selected].0;
                        let labels = &issue.labels;
                        self.action_tx
                            .as_ref()
                            .unwrap()
                            .send(crate::ui::Action::SelectedIssue {
                                number: issue.number,
                                labels: labels.clone(),
                            })
                            .await
                            .unwrap();
                        self.action_tx
                            .as_ref()
                            .unwrap()
                            .send(crate::ui::Action::SelectedIssuePreview {
                                seed: IssuePreviewSeed::from_issue(issue),
                            })
                            .await
                            .unwrap();
                    }
                }
            }
            crate::ui::Action::NewPage(p) => {
                info!("New Page with {} issues", p.items.len());
                self.issues
                    .extend(p.items.iter().cloned().map(IssueListItem::from));
                let count = self.issues.len().min(u32::MAX as usize) as u32;
                LOADED_ISSUE_COUNT.store(count, Ordering::Relaxed);
                self.page = Some(p);
                self.state = State::Loaded;
            }
            crate::ui::Action::FinishedLoading => {
                self.state = State::Loaded;
            }
            crate::ui::Action::IssueLabelsUpdated { number, labels } => {
                if let Some(issue) = self.issues.iter_mut().find(|i| i.0.number == number) {
                    issue.0.labels = labels;
                }
            }
            crate::ui::Action::ChangeIssueScreen(screen) => {
                self.screen = screen;
                if screen == MainScreen::List {
                    self.list_state.focus.set(true);
                }
            }
            _ => {}
        }
    }

    fn should_render(&self) -> bool {
        self.screen == MainScreen::List
    }

    fn is_animating(&self) -> bool {
        self.screen == MainScreen::List && self.state == State::Loading
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

    fn navigable(&self) -> Navigation {
        if self.screen == MainScreen::List {
            Navigation::Regular
        } else {
            Navigation::None
        }
    }
}
