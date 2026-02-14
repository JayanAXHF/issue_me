use crate::{
    app::GITHUB_CLIENT,
    errors::AppError,
    ui::{
        Action, MergeStrategy,
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
    text_input::TextInputState,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style, Stylize},
    symbols,
    widgets::{Block, ListItem, Padding, StatefulWidget},
};
use ratatui_macros::{line, span, vertical};
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};
use textwrap::{Options, wrap};
use throbber_widgets_tui::ThrobberState;
use tokio::sync::oneshot;
use tracing::info;

pub static LOADED_ISSUE_COUNT: AtomicU32 = AtomicU32::new(0);
pub const HELP: &str = "\
↑/↓: Navigate Issues
Enter: View Issue Details
a: Add Assignee(s)
A: Remove Assignee(s)
n: Create New Issue
";
pub struct IssueList<'a> {
    pub issues: Vec<IssueListItem>,
    pub page: Option<Arc<Page<Issue>>>,
    pub list_state: rat_widget::list::ListState<RowSelection>,
    pub handler: IssueHandler<'a>,
    pub action_tx: Option<tokio::sync::mpsc::Sender<crate::ui::Action>>,
    pub throbber_state: ThrobberState,
    pub assign_throbber_state: ThrobberState,
    pub assign_input_state: rat_widget::text_input::TextInputState,
    assign_loading: bool,
    assign_done_rx: Option<oneshot::Receiver<()>>,
    pub owner: String,
    pub repo: String,
    index: usize,
    state: LoadingState,
    inner_state: IssueListState,
    assignment_mode: AssignmentMode,
    pub screen: MainScreen,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum IssueListState {
    #[default]
    Normal,
    AssigningInput,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum AssignmentMode {
    #[default]
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum LoadingState {
    #[default]
    Loading,
    Loaded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MainScreen {
    #[default]
    List,
    Details,
    CreateIssue,
}

impl<'a> IssueList<'a> {
    pub async fn new(
        handler: IssueHandler<'a>,
        owner: String,
        repo: String,
        tx: tokio::sync::mpsc::Sender<Action>,
    ) -> Self {
        LOADED_ISSUE_COUNT.store(0, Ordering::Relaxed);
        let owner_clone = owner.clone();
        let repo_clone = repo.clone();
        tokio::spawn(async move {
            let Ok(mut p) = GITHUB_CLIENT
                .get()
                .unwrap()
                .inner()
                .search()
                .issues_and_pull_requests(&format!(
                    "repo:{}/{} is:issue is:open",
                    owner_clone, repo_clone
                ))
                .page(1u32)
                .per_page(15u8)
                .send()
                .await
            else {
                return;
            };
            let items = std::mem::take(&mut p.items);
            p.items = items;

            tx.send(Action::NewPage(Arc::new(p), MergeStrategy::Append))
                .await
                .unwrap();
        });
        Self {
            page: None,
            owner,
            repo,
            throbber_state: ThrobberState::default(),
            action_tx: None,
            issues: vec![],
            list_state: rat_widget::list::ListState::default(),
            assign_throbber_state: ThrobberState::default(),
            assign_input_state: TextInputState::default(),
            assign_loading: false,
            assign_done_rx: None,
            handler,
            index: 0,
            screen: MainScreen::default(),
            state: LoadingState::default(),
            inner_state: IssueListState::default(),
            assignment_mode: AssignmentMode::default(),
        }
    }
    pub fn render(&mut self, mut area: Layout, buf: &mut Buffer) {
        if self.assign_input_state.lost_focus() {
            self.inner_state = IssueListState::Normal;
        }

        let mut assign_input_area = Rect::default();
        if self.inner_state == IssueListState::AssigningInput {
            let split = vertical![*=1, ==3].split(area.main_content);
            area.main_content = split[0];
            assign_input_area = split[1];
        }
        let mut block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&self.list_state))
            .padding(Padding::horizontal(3));
        if self.state != LoadingState::Loading {
            block = block.title(format!("[{}] Issues", self.index));
        }
        let list = rat_widget::list::List::<RowSelection>::new(
            self.issues.iter().map(Into::<ListItem>::into),
        )
        .block(block)
        .style(Style::default())
        .focus_style(Style::default().reversed().add_modifier(Modifier::BOLD));
        list.render(area.main_content, buf, &mut self.list_state);
        if self.state == LoadingState::Loading {
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
        if self.inner_state == IssueListState::AssigningInput {
            let mut input_block = Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(get_border_style(&self.assign_input_state));
            if !self.assign_loading {
                input_block = input_block.title(match self.assignment_mode {
                    AssignmentMode::Add => "Assign to",
                    AssignmentMode::Remove => "Remove assignee(s)",
                });
            }
            let input = rat_widget::text_input::TextInput::new().block(input_block);
            input.render(assign_input_area, buf, &mut self.assign_input_state);
            if self.assign_loading {
                let title_area = Rect {
                    x: assign_input_area.x + 1,
                    y: assign_input_area.y,
                    width: 10,
                    height: 1,
                };
                let full = throbber_widgets_tui::Throbber::default()
                    .label("Loading")
                    .style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan))
                    .throbber_set(throbber_widgets_tui::BRAILLE_SIX_DOUBLE)
                    .use_type(throbber_widgets_tui::WhichUse::Spin);
                full.render(title_area, buf, &mut self.assign_throbber_state);
            }
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
                if self.state == LoadingState::Loading {
                    self.throbber_state.calc_next();
                }
                if self.assign_loading {
                    self.assign_throbber_state.calc_next();
                }
                if let Some(rx) = self.assign_done_rx.as_mut()
                    && rx.try_recv().is_ok()
                {
                    self.assign_done_rx = None;
                    self.assign_loading = false;
                    self.assign_input_state.set_text("");
                    self.inner_state = IssueListState::Normal;
                    self.list_state.focus.set(true);
                    if let Some(action_tx) = self.action_tx.as_ref() {
                        let _ = action_tx.send(Action::ForceRender).await;
                    }
                }
            }
            crate::ui::Action::AppEvent(ref event) => {
                if self.screen != MainScreen::List {
                    return;
                }
                if matches!(event, ct_event!(key press 'a')) && self.list_state.is_focused() {
                    self.inner_state = IssueListState::AssigningInput;
                    self.assignment_mode = AssignmentMode::Add;
                    self.assign_input_state.set_text("");
                    self.assign_input_state.focus.set(true);
                    self.list_state.focus.set(false);
                    return;
                }
                if matches!(event, ct_event!(key press SHIFT-'A')) && self.list_state.is_focused() {
                    self.inner_state = IssueListState::AssigningInput;
                    self.assignment_mode = AssignmentMode::Remove;
                    self.assign_input_state.set_text("");
                    self.assign_input_state.focus.set(true);
                    self.list_state.focus.set(false);
                    return;
                }
                if matches!(event, ct_event!(key press 'n')) && self.list_state.is_focused() {
                    self.action_tx
                        .as_ref()
                        .unwrap()
                        .send(crate::ui::Action::EnterIssueCreate)
                        .await
                        .unwrap();
                    self.action_tx
                        .as_ref()
                        .unwrap()
                        .send(crate::ui::Action::ChangeIssueScreen(
                            MainScreen::CreateIssue,
                        ))
                        .await
                        .unwrap();
                    return;
                }
                if matches!(event, ct_event!(keycode press Esc))
                    && self.inner_state == IssueListState::AssigningInput
                {
                    self.assign_input_state.set_text("");
                    self.inner_state = IssueListState::Normal;
                    self.list_state.focus.set(true);
                    if let Some(action_tx) = self.action_tx.as_ref() {
                        action_tx.send(Action::ForceRender).await.unwrap();
                    }
                    return;
                }
                if matches!(event, ct_event!(keycode press Enter))
                    && self.inner_state == IssueListState::AssigningInput
                    && !self.assign_loading
                    && let Some(selected) = self.list_state.selected_checked()
                {
                    let issue = &self.issues[selected].0;
                    let value: String = self.assign_input_state.value();
                    let mut assignees = value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect::<Vec<_>>();
                    if !assignees.is_empty() {
                        let tx = self.action_tx.as_ref().unwrap().clone();
                        let (done_tx, done_rx) = oneshot::channel();
                        self.assign_done_rx = Some(done_rx);
                        self.assign_loading = true;
                        let assignment_mode = self.assignment_mode;
                        let number = issue.number;
                        let owner = self.owner.clone();
                        let repo = self.repo.clone();
                        tokio::spawn(async move {
                            let assignees = std::mem::take(&mut assignees);
                            let assignees = assignees
                                .iter()
                                .filter_map(|s| if s.is_empty() { None } else { Some(&**s) })
                                .collect::<Vec<_>>();

                            let issue_handler =
                                GITHUB_CLIENT.get().unwrap().inner().issues(owner, repo);
                            let res = match assignment_mode {
                                AssignmentMode::Add => {
                                    issue_handler
                                        .add_assignees(number, assignees.as_slice())
                                        .await
                                }
                                AssignmentMode::Remove => {
                                    issue_handler
                                        .remove_assignees(number, assignees.as_slice())
                                        .await
                                }
                            };
                            if let Ok(issue) = res {
                                tx.send(crate::ui::Action::SelectedIssuePreview {
                                    seed: IssuePreviewSeed::from_issue(&issue),
                                })
                                .await
                                .unwrap();
                            }
                            let _ = done_tx.send(());
                        });
                    }
                }
                if matches!(event, ct_event!(keycode press Enter)) && self.list_state.is_focused() {
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

                self.assign_input_state
                    .handle(event, rat_widget::event::Regular);
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
                            self.state = LoadingState::Loading;
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
                                    tx.send(crate::ui::Action::NewPage(
                                        Arc::new(p),
                                        MergeStrategy::Append,
                                    ))
                                    .await
                                    .map_err(|_| AppError::TokioMpsc)?;
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
            crate::ui::Action::NewPage(p, merge_strat) => {
                info!("New Page with {} issues", p.items.len());
                match merge_strat {
                    MergeStrategy::Replace => {
                        self.issues = p.items.iter().cloned().map(IssueListItem).collect()
                    }
                    MergeStrategy::Append => self
                        .issues
                        .extend(p.items.iter().cloned().map(IssueListItem)),
                }
                let count = self.issues.len().min(u32::MAX as usize) as u32;
                LOADED_ISSUE_COUNT.store(count, Ordering::Relaxed);
                self.page = Some(p);
                self.state = LoadingState::Loaded;
            }
            crate::ui::Action::FinishedLoading => {
                self.state = LoadingState::Loaded;
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
                } else {
                    self.list_state.focus.set(false);
                }
            }
            _ => {}
        }
    }

    fn should_render(&self) -> bool {
        self.screen == MainScreen::List
    }

    fn is_animating(&self) -> bool {
        self.screen == MainScreen::List
            && (self.state == LoadingState::Loading || self.assign_loading)
    }
    fn set_index(&mut self, index: usize) {
        self.index = index;
    }

    fn set_global_help(&self) {
        info!("Setting global help for IssueList");
        self.action_tx
            .as_ref()
            .unwrap()
            .try_send(crate::ui::Action::SetHelp(HELP))
            .unwrap();
    }
}

impl HasFocus for IssueList<'_> {
    fn build(&self, builder: &mut rat_widget::focus::FocusBuilder) {
        let tag = builder.start(self);
        builder.widget(&self.list_state);
        if self.inner_state == IssueListState::AssigningInput {
            builder.widget(&self.assign_input_state);
        }
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
