use crate::{
    app::GITHUB_CLIENT,
    bookmarks::Bookmarks,
    errors::AppError,
    ui::{
        Action, CloseIssueReason, MergeStrategy,
        components::{
            Component, help::HelpElementKind, issue_conversation::IssueConversationSeed,
            issue_detail::IssuePreviewSeed,
        },
        layout::Layout,
        utils::get_border_style,
    },
};
use anyhow::anyhow;
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
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols,
    text::Line,
    widgets::{
        Block, Clear, List as TuiList, ListItem, ListState as TuiListState, Padding,
        StatefulWidget, Widget,
    },
};
use ratatui_macros::{line, span, vertical};
use ratatui_toaster::{ToastPosition, ToastType};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, RwLock,
        atomic::{AtomicU32, Ordering},
    },
};
use textwrap::{Options, wrap};
use throbber_widgets_tui::{BRAILLE_SIX_DOUBLE, Throbber, ThrobberState, WhichUse};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::trace;

pub static LOADED_ISSUE_COUNT: AtomicU32 = AtomicU32::new(0);
pub const HELP: &[HelpElementKind] = &[
    crate::help_text!("Issue List Help"),
    crate::help_keybind!("Up/Down", "navigate issues"),
    crate::help_keybind!("Enter", "view issue details"),
    crate::help_keybind!("b", "toggle bookmark"),
    crate::help_keybind!("B", "open bookmark finder"),
    crate::help_keybind!("C", "close selected issue"),
    crate::help_keybind!("l", "copy issue link to clipboard"),
    crate::help_keybind!("Enter (bookmark popup)", "open selected bookmark"),
    crate::help_keybind!("Esc (bookmark popup)", "close bookmark popup"),
    crate::help_keybind!("Enter (popup)", "confirm close reason"),
    crate::help_keybind!("a", "add assignee(s)"),
    crate::help_keybind!("A", "remove assignee(s)"),
    crate::help_keybind!("n", "create new issue"),
    crate::help_keybind!("Esc", "cancel popup / assign input"),
];
pub struct IssueList<'a> {
    pub issues: Vec<IssueListItem>,
    pub page: Option<Arc<Page<Issue>>>,
    pub list_state: rat_widget::list::ListState<RowSelection>,
    pub handler: IssueHandler<'a>,
    pub action_tx: Option<tokio::sync::mpsc::Sender<crate::ui::Action>>,
    pub throbber_state: ThrobberState,
    pub assign_throbber_state: ThrobberState,
    pub assign_input_state: rat_widget::text_input::TextInputState,
    bookmarks: Arc<RwLock<Bookmarks>>,
    assign_loading: bool,
    assign_done_rx: Option<oneshot::Receiver<()>>,
    close_popup: Option<IssueClosePopupState>,
    close_error: Option<String>,
    bookmark_popup: Option<BookmarkPopupState>,
    bookmark_titles: HashMap<u64, Arc<str>>,
    bookmark_title_errors: HashMap<u64, Arc<str>>,
    bookmark_error: Option<String>,
    pub owner: String,
    pub repo: String,
    index: usize,
    state: LoadingState,
    inner_state: IssueListState,
    assignment_mode: AssignmentMode,
    pub screen: MainScreen,
}

#[derive(Debug)]
pub(crate) struct IssueClosePopupState {
    pub(crate) issue_number: u64,
    pub(crate) loading: bool,
    pub(crate) throbber_state: ThrobberState,
    pub(crate) error: Option<String>,
    reason_state: TuiListState,
}

#[derive(Debug)]
struct BookmarkPopupState {
    issue_numbers: Vec<u64>,
    state: TuiListState,
    loading_numbers: HashSet<u64>,
    fetch_cancel: CancellationToken,
    throbber_state: ThrobberState,
    opening_issue: Option<u64>,
}

impl IssueClosePopupState {
    pub(crate) fn new(issue_number: u64) -> Self {
        let mut reason_state = TuiListState::default();
        reason_state.select(Some(0));
        Self {
            issue_number,
            loading: false,
            throbber_state: ThrobberState::default(),
            error: None,
            reason_state,
        }
    }

    pub(crate) fn select_next_reason(&mut self) {
        self.reason_state.select_next();
    }

    pub(crate) fn select_prev_reason(&mut self) {
        self.reason_state.select_previous();
    }

    pub(crate) fn selected_reason(&self) -> CloseIssueReason {
        self.reason_state
            .selected()
            .and_then(|idx| CloseIssueReason::ALL.get(idx).copied())
            .unwrap_or(CloseIssueReason::Completed)
    }
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
    DetailsFullscreen,
    CreateIssue,
}

impl<'a> IssueList<'a> {
    pub async fn new(
        handler: IssueHandler<'a>,
        owner: String,
        repo: String,
        tx: tokio::sync::mpsc::Sender<Action>,
        bookmarks: Arc<RwLock<Bookmarks>>,
    ) -> Self {
        LOADED_ISSUE_COUNT.store(0, Ordering::Relaxed);
        let owner_clone = owner.clone();
        let repo_clone = repo.clone();
        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                return;
            };
            let Ok(mut p) = client
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

            let _ = tx
                .send(Action::NewPage(Arc::new(p), MergeStrategy::Append))
                .await;
        });
        Self {
            page: None,
            owner,
            bookmarks,
            repo,
            throbber_state: ThrobberState::default(),
            action_tx: None,
            issues: vec![],
            list_state: rat_widget::list::ListState::default(),
            assign_throbber_state: ThrobberState::default(),
            assign_input_state: TextInputState::default(),
            assign_loading: false,
            assign_done_rx: None,
            close_popup: None,
            close_error: None,
            bookmark_popup: None,
            bookmark_titles: HashMap::new(),
            bookmark_title_errors: HashMap::new(),
            bookmark_error: None,
            handler,
            index: 0,
            screen: MainScreen::default(),
            state: LoadingState::default(),
            inner_state: IssueListState::default(),
            assignment_mode: AssignmentMode::default(),
        }
    }

    fn open_close_popup(&mut self) {
        let Some(selected) = self.list_state.selected_checked() else {
            self.close_error = Some("No issue selected.".to_string());
            return;
        };
        let Some(issue) = self.issues.get(selected).map(|item| &item.0) else {
            self.close_error = Some("No issue selected.".to_string());
            return;
        };
        if issue.state == IssueState::Closed {
            self.close_error = Some("Selected issue is already closed.".to_string());
            return;
        }
        self.close_error = None;
        self.close_popup = Some(IssueClosePopupState::new(issue.number));
    }

    fn render_close_popup(&mut self, area: Rect, buf: &mut Buffer) {
        let Some(popup) = self.close_popup.as_mut() else {
            return;
        };
        render_issue_close_popup(popup, area, buf);
    }

    async fn submit_close_popup(&mut self) {
        let Some(popup) = self.close_popup.as_mut() else {
            return;
        };
        if popup.loading {
            return;
        }
        let reason = popup.selected_reason();
        let number = popup.issue_number;
        popup.loading = true;
        popup.error = None;

        let Some(action_tx) = self.action_tx.clone() else {
            popup.loading = false;
            popup.error = Some("Action channel unavailable.".to_string());
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::IssueCloseError {
                        number,
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let issues = client.inner().issues(owner, repo);
            match issues
                .update(number)
                .state(IssueState::Closed)
                .state_reason(reason.to_octocrab())
                .send()
                .await
            {
                Ok(issue) => {
                    let _ = action_tx
                        .send(Action::IssueCloseSuccess {
                            issue: Box::new(issue),
                        })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::IssueCloseError {
                            number,
                            message: err.to_string().replace('\n', " "),
                        })
                        .await;
                }
            }
        });
    }

    async fn handle_close_popup_event(&mut self, event: &crossterm::event::Event) -> bool {
        let Some(popup) = self.close_popup.as_mut() else {
            return false;
        };
        if popup.loading {
            if matches!(event, ct_event!(keycode press Esc)) {
                popup.loading = false;
            }
            return true;
        }
        if matches!(event, ct_event!(keycode press Esc)) {
            self.close_popup = None;
            return true;
        }
        if matches!(event, ct_event!(keycode press Up)) {
            popup.select_prev_reason();
            return true;
        }
        if matches!(event, ct_event!(keycode press Down)) {
            popup.select_next_reason();
            return true;
        }
        if matches!(event, ct_event!(keycode press Enter)) {
            self.submit_close_popup().await;
            return true;
        }
        true
    }

    fn open_bookmark_popup(&mut self) {
        let mut issue_numbers = {
            let bookmarks = self.bookmarks.read().expect("bookmarks lock poisoned");
            bookmarks.get_bookmarked_issues(&self.owner, &self.repo)
        };
        if issue_numbers.is_empty() {
            self.bookmark_error = Some("No bookmarks found for this repository.".to_string());
            return;
        }

        issue_numbers.sort_unstable();
        let mut state = TuiListState::default();
        state.select(Some(0));
        self.list_state.focus.set(false);
        self.bookmark_error = None;
        self.bookmark_popup = Some(BookmarkPopupState {
            issue_numbers,
            state,
            loading_numbers: HashSet::new(),
            fetch_cancel: CancellationToken::new(),
            throbber_state: ThrobberState::default(),
            opening_issue: None,
        });
        self.ensure_bookmark_titles_for_window();
    }

    fn close_bookmark_popup(&mut self) {
        if let Some(popup) = self.bookmark_popup.take() {
            popup.fetch_cancel.cancel();
        }
        if self.screen == MainScreen::List {
            self.list_state.focus.set(true);
        }
    }

    fn selected_bookmark_number(&self) -> Option<u64> {
        let popup = self.bookmark_popup.as_ref()?;
        let selected = popup.state.selected()?;
        popup.issue_numbers.get(selected).copied()
    }

    fn ensure_bookmark_titles_for_window(&mut self) {
        let Some(popup) = self.bookmark_popup.as_ref() else {
            return;
        };
        if popup.issue_numbers.is_empty() {
            return;
        }
        let selected = popup.state.selected().unwrap_or(0);
        let start = selected.saturating_sub(4);
        let end = selected
            .saturating_add(5)
            .min(popup.issue_numbers.len().saturating_sub(1));
        let to_request = popup.issue_numbers[start..=end]
            .iter()
            .copied()
            .filter(|number| {
                !self.bookmark_titles.contains_key(number)
                    && !self.bookmark_title_errors.contains_key(number)
                    && !popup.loading_numbers.contains(number)
            })
            .collect::<Vec<_>>();
        for number in to_request {
            self.fetch_bookmark_title(number);
        }
    }

    fn fetch_bookmark_title(&mut self, number: u64) {
        let Some(popup) = self.bookmark_popup.as_mut() else {
            return;
        };
        if !popup.loading_numbers.insert(number) {
            return;
        }
        let Some(action_tx) = self.action_tx.clone() else {
            popup.loading_numbers.remove(&number);
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let cancel = popup.fetch_cancel.clone();
        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::BookmarkTitleLoadError {
                        number,
                        message: Arc::<str>::from("GitHub client not initialized."),
                    })
                    .await;
                return;
            };
            let issues = client.inner().issues(owner, repo);
            let title_result = tokio::select! {
                _ = cancel.cancelled() => {
                    return;
                }
                result = issues.get(number) => {
                    result
                }
            };

            match title_result {
                Ok(issue) => {
                    let _ = action_tx
                        .send(Action::BookmarkTitleLoaded {
                            number,
                            title: Arc::<str>::from(issue.title),
                        })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::BookmarkTitleLoadError {
                            number,
                            message: Arc::<str>::from(err.to_string().replace('\n', " ")),
                        })
                        .await;
                }
            }
        });
    }

    async fn open_selected_bookmark(&mut self) -> Result<(), AppError> {
        let Some(number) = self.selected_bookmark_number() else {
            return Ok(());
        };

        if let Some(issue) = self
            .issues
            .iter()
            .find(|i| i.0.number == number)
            .map(|i| &i.0)
        {
            let labels = issue.labels.clone();
            let preview_seed = IssuePreviewSeed::from_issue(issue);
            let conversation_seed = IssueConversationSeed::from_issue(issue);
            self.close_bookmark_popup();
            if let Some(action_tx) = self.action_tx.as_ref() {
                action_tx
                    .send(Action::SelectedIssue { number, labels })
                    .await?;
                action_tx
                    .send(Action::SelectedIssuePreview { seed: preview_seed })
                    .await?;
                action_tx
                    .send(Action::EnterIssueDetails {
                        seed: conversation_seed,
                    })
                    .await?;
                action_tx
                    .send(Action::ChangeIssueScreen(MainScreen::Details))
                    .await?;
            }
            return Ok(());
        }

        let Some(popup) = self.bookmark_popup.as_mut() else {
            return Ok(());
        };
        if popup.opening_issue == Some(number) {
            return Ok(());
        }
        popup.opening_issue = Some(number);
        let Some(action_tx) = self.action_tx.clone() else {
            popup.opening_issue = None;
            return Ok(());
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let cancel = popup.fetch_cancel.clone();
        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::BookmarkedIssueLoadError {
                        number,
                        message: Arc::<str>::from("GitHub client not initialized."),
                    })
                    .await;
                return;
            };
            let issues = client.inner().issues(owner, repo);
            let issue_result = tokio::select! {
                _ = cancel.cancelled() => {
                    return;
                }
                result = issues.get(number) => {
                    result
                }
            };
            match issue_result {
                Ok(issue) => {
                    let _ = action_tx
                        .send(Action::BookmarkedIssueLoaded {
                            issue: Box::new(issue),
                        })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::BookmarkedIssueLoadError {
                            number,
                            message: Arc::<str>::from(err.to_string().replace('\n', " ")),
                        })
                        .await;
                }
            }
        });
        Ok(())
    }

    async fn handle_bookmark_popup_event(
        &mut self,
        event: &crossterm::event::Event,
    ) -> Result<bool, AppError> {
        let Some(_) = self.bookmark_popup.as_ref() else {
            return Ok(false);
        };

        if matches!(event, ct_event!(keycode press Esc)) {
            self.close_bookmark_popup();
            return Ok(true);
        }
        if matches!(event, ct_event!(keycode press Enter)) {
            self.open_selected_bookmark().await?;
            return Ok(true);
        }

        if let Some(popup) = self.bookmark_popup.as_mut() {
            if matches!(event, ct_event!(keycode press Up)) {
                popup.state.select_previous();
                self.ensure_bookmark_titles_for_window();
                return Ok(true);
            }
            if matches!(event, ct_event!(keycode press Down)) {
                popup.state.select_next();
                self.ensure_bookmark_titles_for_window();
                return Ok(true);
            }
            return Ok(true);
        }

        Ok(true)
    }

    fn render_bookmark_popup_item(
        number: u64,
        width: usize,
        bookmark_titles: &HashMap<u64, Arc<str>>,
        bookmark_title_errors: &HashMap<u64, Arc<str>>,
    ) -> ListItem<'static> {
        let width = width.max(10);
        let (content, style) = if let Some(title) = bookmark_titles.get(&number) {
            (format!("#{number} {title}"), Style::default())
        } else if let Some(err) = bookmark_title_errors.get(&number) {
            (
                format!("#{number} Failed to load title: {err}"),
                Style::default().fg(Color::LightRed),
            )
        } else {
            (format!("#{number} Title pending"), Style::default().dim())
        };

        let lines = wrap(content.as_str(), Options::new(width))
            .into_iter()
            .map(|line| Line::from(line.into_owned()))
            .collect::<Vec<_>>();
        ListItem::new(lines).style(style)
    }

    fn render_bookmark_popup(&mut self, area: Rect, buf: &mut Buffer) {
        let Some(popup) = self.bookmark_popup.as_mut() else {
            return;
        };

        let popup_area = area.centered(Constraint::Percentage(50), Constraint::Percentage(30));
        Clear.render(popup_area, buf);
        let mut title = "Bookmarks | Enter: open Esc: close".to_string();
        if !popup.loading_numbers.is_empty() {
            title.push_str(&format!(" | Loading {}", popup.loading_numbers.len()));
        }
        if let Some(number) = popup.opening_issue {
            title.push_str(&format!(" | Opening #{number}..."));
        }
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .title(title);
        let inner = block.inner(popup_area);

        let wrap_width = inner.width.saturating_sub(3).max(10) as usize;
        let title_cache = &self.bookmark_titles;
        let title_errors = &self.bookmark_title_errors;
        let list = TuiList::new(popup.issue_numbers.iter().copied().map(|number| {
            Self::render_bookmark_popup_item(number, wrap_width, title_cache, title_errors)
        }))
        .highlight_style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(block)
        .highlight_symbol("> ");
        StatefulWidget::render(list, popup_area, buf, &mut popup.state);

        if !popup.loading_numbers.is_empty() {
            let title_area = Rect {
                x: popup_area.x + 1,
                y: popup_area.y,
                width: 10,
                height: 1,
            };
            let throbber = Throbber::default()
                .label("Loading")
                .style(Style::new().fg(Color::Cyan))
                .throbber_set(BRAILLE_SIX_DOUBLE)
                .use_type(WhichUse::Spin);
            StatefulWidget::render(throbber, title_area, buf, &mut popup.throbber_state);
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
            let mut title = format!("[{}] Issues", self.index);
            if let Some(err) = &self.close_error {
                title.push_str(" | ");
                title.push_str(err);
            } else if let Some(err) = &self.bookmark_error {
                title.push_str(" | ");
                title.push_str(err);
            }
            block = block.title(title);
        }
        {
            let bookmarks = self.bookmarks.read().unwrap();
            let list = rat_widget::list::List::<RowSelection>::new(
                self.issues
                    .iter()
                    .map(|issue| self.build_list_item(issue, &bookmarks)),
            )
            .block(block)
            .style(Style::default())
            .focus_style(Style::default().reversed().add_modifier(Modifier::BOLD));
            list.render(area.main_content, buf, &mut self.list_state);
        }
        if self.state == LoadingState::Loading {
            let title_area = Rect {
                x: area.main_content.x + 1,
                y: area.main_content.y,
                width: 10,
                height: 1,
            };
            let full = Throbber::default()
                .label("Loading")
                .style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan))
                .throbber_set(BRAILLE_SIX_DOUBLE)
                .use_type(WhichUse::Spin);
            StatefulWidget::render(full, title_area, buf, &mut self.throbber_state);
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
                let full = Throbber::default()
                    .label("Loading")
                    .style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan))
                    .throbber_set(BRAILLE_SIX_DOUBLE)
                    .use_type(WhichUse::Spin);
                StatefulWidget::render(full, title_area, buf, &mut self.assign_throbber_state);
            }
        }
        self.render_close_popup(area.main_content, buf);
        self.render_bookmark_popup(area.main_content, buf);
    }

    fn build_list_item(&self, issue: &'a IssueListItem, bookmarks: &Bookmarks) -> ListItem<'a> {
        let options = Options::with_termwidth();
        let binding = issue.body.clone().unwrap_or("No desc provided".to_string());
        let mut body = wrap(binding.trim(), options);
        body.truncate(2);

        let bookmarked = bookmarks.is_bookmarked(&self.owner, &self.repo, issue.number);
        let bookmark_symbol = if bookmarked { " b " } else { "   " };

        let lines = vec![
            line![
                span!(bookmark_symbol).style(if bookmarked {
                    Style::new().reversed()
                } else {
                    Style::new()
                }),
                span!(issue.title.as_str()),
                " ",
                span!("#{}", issue.number).dim(),
            ],
            line![
                span!(symbols::shade::FULL).style({
                    if matches!(issue.state, IssueState::Open) {
                        Style::new().green()
                    } else {
                        Style::new().magenta()
                    }
                }),
                "  ",
                span!(
                    "Opened by {} at {}",
                    issue.user.login,
                    issue.created_at.format("%Y-%m-%d %H:%M:%S")
                )
                .dim(),
            ],
            line!["   ", span!(body.join(" ")).style(Style::new().dim())],
        ];
        ListItem::new(lines)
    }
}

pub(crate) fn render_issue_close_popup(
    popup: &mut IssueClosePopupState,
    area: Rect,
    buf: &mut Buffer,
) {
    let popup_area = area.centered(Constraint::Percentage(20), Constraint::Length(5));
    Clear.render(popup_area, buf);

    let mut block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .title_bottom("Enter: close  Esc: cancel")
        .title(format!("Close issue #{}", popup.issue_number));
    if let Some(err) = &popup.error {
        block = block.title(format!("Close issue #{} | {}", popup.issue_number, err));
    }
    let inner = block.inner(popup_area);
    block.render(popup_area, buf);

    if popup.reason_state.selected().is_none() {
        popup.reason_state.select(Some(0));
    }
    let items = CloseIssueReason::ALL
        .iter()
        .map(|reason| ListItem::new(reason.label()))
        .collect::<Vec<_>>();
    let list = TuiList::new(items)
        .highlight_style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");
    StatefulWidget::render(list, inner, buf, &mut popup.reason_state);

    if popup.loading {
        let title_area = Rect {
            x: popup_area.x + 1,
            y: popup_area.y,
            width: 10,
            height: 1,
        };
        let throbber = Throbber::default()
            .label("Closing")
            .style(Style::new().fg(Color::Cyan))
            .throbber_set(BRAILLE_SIX_DOUBLE)
            .use_type(WhichUse::Spin);
        StatefulWidget::render(throbber, title_area, buf, &mut popup.throbber_state);
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

#[async_trait(?Send)]
impl Component for IssueList<'_> {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }

    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<crate::ui::Action>) {
        self.action_tx = Some(action_tx);
    }

    async fn handle_event(&mut self, event: crate::ui::Action) -> Result<(), AppError> {
        match event {
            crate::ui::Action::Tick => {
                if self.state == LoadingState::Loading {
                    self.throbber_state.calc_next();
                }
                if self.assign_loading {
                    self.assign_throbber_state.calc_next();
                }
                if let Some(popup) = self.close_popup.as_mut()
                    && popup.loading
                {
                    popup.throbber_state.calc_next();
                }
                if let Some(popup) = self.bookmark_popup.as_mut()
                    && !popup.loading_numbers.is_empty()
                {
                    popup.throbber_state.calc_next();
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
                    return Ok(());
                }
                if self.handle_bookmark_popup_event(event).await? {
                    return Ok(());
                }
                if self.handle_close_popup_event(event).await {
                    return Ok(());
                }

                match event {
                    ct_event!(key press 'a') if self.list_state.is_focused() => {
                        self.inner_state = IssueListState::AssigningInput;
                        self.assignment_mode = AssignmentMode::Add;
                        self.assign_input_state.set_text("");
                        self.assign_input_state.focus.set(true);
                        self.list_state.focus.set(false);
                        return Ok(());
                    }
                    ct_event!(key press SHIFT-'A') if self.list_state.is_focused() => {
                        self.inner_state = IssueListState::AssigningInput;
                        self.assignment_mode = AssignmentMode::Remove;
                        self.assign_input_state.set_text("");
                        self.assign_input_state.focus.set(true);
                        self.list_state.focus.set(false);
                        return Ok(());
                    }
                    ct_event!(key press SHIFT-'B') if self.list_state.is_focused() => {
                        if self.bookmark_popup.is_some() {
                            self.close_bookmark_popup();
                        } else {
                            self.open_bookmark_popup();
                        }
                        return Ok(());
                    }
                    ct_event!(key press 'b') => {
                        if let Some(selected) = self.list_state.selected_checked() {
                            let issue = &self.issues[selected].0;
                            {
                                let mut bookmarks =
                                    self.bookmarks.write().expect("bookmarks lock poisoned");
                                if bookmarks.is_bookmarked(&self.owner, &self.repo, issue.number) {
                                    bookmarks.remove(&self.owner, &self.repo, issue.number);
                                } else {
                                    bookmarks.add(&self.owner, &self.repo, issue.number);
                                }
                            }
                            if let Some(action_tx) = self.action_tx.as_ref() {
                                let _ = action_tx.send(Action::ForceRender).await;
                            }
                        }
                    }
                    ct_event!(key press 'n') if self.list_state.is_focused() => {
                        self.action_tx
                            .as_ref()
                            .ok_or_else(|| {
                                AppError::Other(anyhow!("issue list action channel unavailable"))
                            })?
                            .send(crate::ui::Action::EnterIssueCreate)
                            .await?;
                        self.action_tx
                            .as_ref()
                            .ok_or_else(|| {
                                AppError::Other(anyhow!("issue list action channel unavailable"))
                            })?
                            .send(crate::ui::Action::ChangeIssueScreen(
                                MainScreen::CreateIssue,
                            ))
                            .await?;
                        return Ok(());
                    }
                    ct_event!(key press SHIFT-'C')
                        if self.list_state.is_focused()
                            && self.inner_state == IssueListState::Normal =>
                    {
                        self.open_close_popup();
                        return Ok(());
                    }
                    ct_event!(keycode press Esc)
                        if self.inner_state == IssueListState::AssigningInput =>
                    {
                        self.assign_input_state.set_text("");
                        self.inner_state = IssueListState::Normal;
                        self.list_state.focus.set(true);
                        if let Some(action_tx) = self.action_tx.as_ref() {
                            action_tx.send(Action::ForceRender).await?;
                        }
                        return Ok(());
                    }

                    ct_event!(key press 'l') if self.list_state.is_focused() => {
                        let Some(selected) = self.list_state.selected_checked() else {
                            return Ok(());
                        };
                        let issue = &self.issues[selected].0;
                        let link = format!(
                            "https://github.com/{}/{}/issues/{}",
                            self.owner, self.repo, issue.number
                        );

                        cli_clipboard::set_contents(link)
                            .map_err(|_| anyhow!("Error copying to clipboard"))?;
                        if let Some(tx) = self.action_tx.as_ref() {
                            tx.send(Action::ToastAction(ratatui_toaster::ToastMessage::Show {
                                message: "Copied Link to Clipboard".to_string(),
                                toast_type: ToastType::Success,
                                position: ToastPosition::TopRight,
                            }))
                            .await?;
                            tx.send(Action::ForceRender).await?;
                        }
                    }

                    _ => {}
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
                        let tx = self
                            .action_tx
                            .as_ref()
                            .ok_or_else(|| {
                                AppError::Other(anyhow!("issue list action channel unavailable"))
                            })?
                            .clone();
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

                            let issue_handler = if let Some(client) = GITHUB_CLIENT.get() {
                                client.inner().issues(owner, repo)
                            } else {
                                let _ = done_tx.send(());
                                return;
                            };
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
                                let _ = tx
                                    .send(crate::ui::Action::SelectedIssuePreview {
                                        seed: IssuePreviewSeed::from_issue(&issue),
                                    })
                                    .await;
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
                            .ok_or_else(|| {
                                AppError::Other(anyhow!("issue list action channel unavailable"))
                            })?
                            .send(crate::ui::Action::EnterIssueDetails {
                                seed: IssueConversationSeed::from_issue(issue),
                            })
                            .await?;
                        self.action_tx
                            .as_ref()
                            .ok_or_else(|| {
                                AppError::Other(anyhow!("issue list action channel unavailable"))
                            })?
                            .send(crate::ui::Action::ChangeIssueScreen(MainScreen::Details))
                            .await?;
                    }
                    return Ok(());
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
                            let tx = self
                                .action_tx
                                .as_ref()
                                .ok_or_else(|| {
                                    AppError::Other(anyhow!(
                                        "issue list action channel unavailable"
                                    ))
                                })?
                                .clone();
                            let page_next = page.next.clone();
                            self.state = LoadingState::Loading;
                            tokio::spawn(async move {
                                let Some(client) = GITHUB_CLIENT.get() else {
                                    let _ = tx.send(crate::ui::Action::FinishedLoading).await;
                                    return;
                                };
                                let p = client.inner().get_page::<Issue>(&page_next).await;
                                if let Ok(pres) = p
                                    && let Some(mut p) = pres
                                {
                                    let items = std::mem::take(&mut p.items);
                                    let items = items
                                        .into_iter()
                                        .filter(|i| i.pull_request.is_none())
                                        .collect();
                                    p.items = items;
                                    let _ = tx
                                        .send(crate::ui::Action::NewPage(
                                            Arc::new(p),
                                            MergeStrategy::Append,
                                        ))
                                        .await;
                                }
                                let _ = tx.send(crate::ui::Action::FinishedLoading).await;
                            });
                        }
                        let issue = &self.issues[selected].0;
                        let labels = &issue.labels;
                        self.action_tx
                            .as_ref()
                            .ok_or_else(|| {
                                AppError::Other(anyhow!("issue list action channel unavailable"))
                            })?
                            .send(crate::ui::Action::SelectedIssue {
                                number: issue.number,
                                labels: labels.clone(),
                            })
                            .await?;
                        self.action_tx
                            .as_ref()
                            .ok_or_else(|| {
                                AppError::Other(anyhow!("issue list action channel unavailable"))
                            })?
                            .send(crate::ui::Action::SelectedIssuePreview {
                                seed: IssuePreviewSeed::from_issue(issue),
                            })
                            .await?;
                    }
                }
            }
            crate::ui::Action::NewPage(p, merge_strat) => {
                trace!("New Page with {} issues", p.items.len());
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
            crate::ui::Action::IssueCloseSuccess { issue } => {
                let issue = *issue;
                if let Some(existing) = self.issues.iter_mut().find(|i| i.0.number == issue.number)
                {
                    existing.0 = issue.clone();
                }
                let initiated_here = self
                    .close_popup
                    .as_ref()
                    .is_some_and(|popup| popup.issue_number == issue.number);
                if initiated_here {
                    self.close_popup = None;
                    self.close_error = None;
                    if let Some(action_tx) = self.action_tx.as_ref() {
                        let _ = action_tx
                            .send(Action::SelectedIssuePreview {
                                seed: IssuePreviewSeed::from_issue(&issue),
                            })
                            .await;
                        let _ = action_tx.send(Action::RefreshIssueList).await;
                    }
                }
            }
            crate::ui::Action::IssueCloseError { number, message } => {
                if let Some(popup) = self.close_popup.as_mut()
                    && popup.issue_number == number
                {
                    popup.loading = false;
                    popup.error = Some(message.clone());
                    self.close_error = Some(message);
                }
            }
            crate::ui::Action::IssueLabelsUpdated { number, labels } => {
                if let Some(issue) = self.issues.iter_mut().find(|i| i.0.number == number) {
                    issue.0.labels = labels;
                }
            }
            crate::ui::Action::BookmarkTitleLoaded { number, title } => {
                self.bookmark_titles.insert(number, title);
                self.bookmark_title_errors.remove(&number);
                if let Some(popup) = self.bookmark_popup.as_mut() {
                    popup.loading_numbers.remove(&number);
                }
            }
            crate::ui::Action::BookmarkTitleLoadError { number, message } => {
                self.bookmark_title_errors.insert(number, message);
                if let Some(popup) = self.bookmark_popup.as_mut() {
                    popup.loading_numbers.remove(&number);
                }
            }
            crate::ui::Action::BookmarkedIssueLoaded { issue } => {
                let issue = *issue;
                let should_open = self
                    .bookmark_popup
                    .as_ref()
                    .is_some_and(|popup| popup.opening_issue == Some(issue.number));
                if !should_open {
                    return Ok(());
                }

                let number = issue.number;
                let labels = issue.labels.clone();
                let preview_seed = IssuePreviewSeed::from_issue(&issue);
                let conversation_seed = IssueConversationSeed::from_issue(&issue);
                self.close_bookmark_popup();

                if let Some(action_tx) = self.action_tx.as_ref() {
                    action_tx
                        .send(Action::SelectedIssue { number, labels })
                        .await?;
                    action_tx
                        .send(Action::SelectedIssuePreview { seed: preview_seed })
                        .await?;
                    action_tx
                        .send(Action::EnterIssueDetails {
                            seed: conversation_seed,
                        })
                        .await?;
                    action_tx
                        .send(Action::ChangeIssueScreen(MainScreen::Details))
                        .await?;
                }
            }
            crate::ui::Action::BookmarkedIssueLoadError { number, message } => {
                if let Some(popup) = self.bookmark_popup.as_mut()
                    && popup.opening_issue == Some(number)
                {
                    popup.opening_issue = None;
                    self.bookmark_error = Some(message.to_string());
                }
            }
            crate::ui::Action::ChangeIssueScreen(screen) => {
                self.screen = screen;
                if screen == MainScreen::List {
                    self.list_state.focus.set(true);
                } else {
                    self.close_popup = None;
                    self.close_bookmark_popup();
                    self.list_state.focus.set(false);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn should_render(&self) -> bool {
        self.screen == MainScreen::List
    }

    fn is_animating(&self) -> bool {
        self.screen == MainScreen::List
            && (self.state == LoadingState::Loading
                || self.assign_loading
                || self.close_popup.as_ref().is_some_and(|popup| popup.loading)
                || self
                    .bookmark_popup
                    .as_ref()
                    .is_some_and(|popup| !popup.loading_numbers.is_empty()))
    }
    fn set_index(&mut self, index: usize) {
        self.index = index;
    }

    fn set_global_help(&self) {
        trace!("Setting global help for IssueList");
        if let Some(action_tx) = self.action_tx.as_ref() {
            let _ = action_tx.try_send(crate::ui::Action::SetHelp(HELP));
        }
    }

    fn capture_focus_event(&self, _event: &crossterm::event::Event) -> bool {
        self.close_popup.is_some() || self.bookmark_popup.is_some()
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
