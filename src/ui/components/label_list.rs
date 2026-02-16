use std::{
    cmp::min,
    slice,
    str::FromStr,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use octocrab::Error as OctoError;
use octocrab::models::Label;
use rat_cursor::HasScreenCursor;
use rat_widget::{
    event::{HandleEvent, Regular, ct_event},
    focus::HasFocus,
    list::{ListState, selection::RowSelection},
    text_input::{TextInput, TextInputState},
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout as TuiLayout},
    style::{Color, Style, Stylize},
    widgets::{Block, Clear, ListItem, Paragraph, StatefulWidget, Widget},
};
use ratatui_macros::{line, span};
use regex::RegexBuilder;
use throbber_widgets_tui::{BRAILLE_SIX_DOUBLE, Throbber, ThrobberState, WhichUse};
use tracing::error;

use crate::{
    app::GITHUB_CLIENT,
    errors::AppError,
    ui::{
        Action, AppState, COLOR_PROFILE,
        components::{Component, help::HelpElementKind, issue_list::MainScreen},
        layout::Layout,
        utils::get_border_style,
    },
};

const MARKER: &str = ratatui::symbols::marker::DOT;
const STATUS_TTL: Duration = Duration::from_secs(3);
const DEFAULT_COLOR: &str = "ededed";
pub const HELP: &[HelpElementKind] = &[
    crate::help_text!("Label List Help"),
    crate::help_keybind!("Up/Down", "select label"),
    crate::help_keybind!("a", "add label to selected issue"),
    crate::help_keybind!("d", "remove selected label from issue"),
    crate::help_keybind!("f", "open popup label regex search"),
    crate::help_keybind!("Ctrl+I", "toggle case-insensitive search (popup)"),
    crate::help_keybind!("Enter", "submit add/create input"),
    crate::help_keybind!("Esc", "cancel current label edit flow"),
    crate::help_keybind!("y / n", "confirm or cancel creating missing label"),
];

#[derive(Debug)]
pub struct LabelList {
    state: ListState<RowSelection>,
    labels: Vec<LabelListItem>,
    action_tx: Option<tokio::sync::mpsc::Sender<Action>>,
    current_issue_number: Option<u64>,
    mode: LabelEditMode,
    status_message: Option<StatusMessage>,
    pending_status: Option<String>,
    owner: String,
    repo: String,
    screen: MainScreen,
    popup_search: Option<PopupLabelSearchState>,
    label_search_request_seq: u64,
    index: usize,
}

#[derive(Debug, Clone)]
struct LabelListItem(Label);

#[derive(Debug)]
enum LabelEditMode {
    Idle,
    Adding { input: TextInputState },
    ConfirmCreate { name: String },
    CreateColor { name: String, input: TextInputState },
}

#[derive(Debug)]
struct PopupLabelSearchState {
    input: TextInputState,
    list_state: ListState<RowSelection>,
    matches: Vec<LabelListItem>,
    loading: bool,
    case_insensitive: bool,
    request_id: u64,
    scanned_count: u32,
    matched_count: u32,
    error: Option<String>,
    throbber_state: ThrobberState,
}

#[derive(Debug, Clone)]
struct StatusMessage {
    message: String,
    at: Instant,
}

impl From<Label> for LabelListItem {
    fn from(value: Label) -> Self {
        Self(value)
    }
}

impl std::ops::Deref for LabelListItem {
    type Target = Label;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&LabelListItem> for ListItem<'_> {
    fn from(value: &LabelListItem) -> Self {
        let rgb = &value.0.color;
        let mut c = Color::from_str(&format!("#{}", rgb)).unwrap_or(Color::Gray);
        if let Some(profile) = COLOR_PROFILE.get() {
            let adapted = profile.adapt_color(c);
            if let Some(adapted) = adapted {
                c = adapted;
            }
        }
        let line = line![span!("{} {}", MARKER, value.0.name).fg(c)];
        ListItem::new(line)
    }
}

fn popup_list_item(value: &LabelListItem) -> ListItem<'_> {
    let rgb = &value.0.color;
    let mut c = Color::from_str(&format!("#{}", rgb)).unwrap_or(Color::Gray);
    if let Some(profile) = COLOR_PROFILE.get() {
        let adapted = profile.adapt_color(c);
        if let Some(adapted) = adapted {
            c = adapted;
        }
    }

    let description = value
        .0
        .description
        .as_deref()
        .filter(|desc| !desc.trim().is_empty())
        .unwrap_or("No description");
    let lines = vec![
        line![span!("{} {}", MARKER, value.0.name).fg(c)],
        line![span!("  {description}").dim()],
    ];
    ListItem::new(lines)
}

impl LabelList {
    pub fn new(AppState { repo, owner, .. }: AppState) -> Self {
        Self {
            state: Default::default(),
            labels: vec![],
            action_tx: None,
            current_issue_number: None,
            mode: LabelEditMode::Idle,
            status_message: None,
            pending_status: None,
            owner,
            repo,
            screen: MainScreen::default(),
            popup_search: None,
            label_search_request_seq: 0,
            index: 0,
        }
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.expire_status();

        let mut list_area = area.label_list;
        let mut footer_area = None;
        if self.needs_footer() {
            let areas = TuiLayout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)])
                .split(area.label_list);
            list_area = areas[0];
            footer_area = Some(areas[1]);
        }

        let title = if let Some(status) = &self.status_message {
            error!("Label list status: {}", status.message);
            format!(
                "[{}] Labels (a:add d:remove) | {}",
                self.index, status.message
            )
        } else {
            format!("[{}] Labels (a:add d:remove)", self.index)
        };
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .title(title)
            .border_style(get_border_style(&self.state));
        let list = rat_widget::list::List::<RowSelection>::new(
            self.labels.iter().map(Into::<ListItem>::into),
        )
        .select_style(Style::default().bg(Color::Black))
        .focus_style(Style::default().bold().bg(Color::Black))
        .block(block);
        list.render(list_area, buf, &mut self.state);

        if let Some(area) = footer_area {
            match &mut self.mode {
                LabelEditMode::Adding { input } => {
                    let widget = TextInput::new().block(
                        Block::bordered()
                            .border_type(ratatui::widgets::BorderType::Rounded)
                            .border_style(get_border_style(input))
                            .title("Add label"),
                    );
                    widget.render(area, buf, input);
                }
                LabelEditMode::ConfirmCreate { name } => {
                    let prompt = format!("Label \"{name}\" not found. Create? (y/n)");
                    Paragraph::new(prompt)
                        .block(
                            Block::bordered()
                                .border_type(ratatui::widgets::BorderType::Rounded)
                                .border_style(Style::default().yellow())
                                .title("Confirm [y/n]"),
                        )
                        .render(area, buf);
                }
                LabelEditMode::CreateColor { input, .. } => {
                    let widget = TextInput::new().block(
                        Block::bordered()
                            .border_type(ratatui::widgets::BorderType::Rounded)
                            .border_style(get_border_style(input))
                            .title("Label color (#RRGGBB)"),
                    );
                    widget.render(area, buf, input);
                }
                LabelEditMode::Idle => {
                    if let Some(status) = &self.status_message {
                        Paragraph::new(status.message.clone()).render(area, buf);
                    }
                }
            }
        }

        self.render_popup(area, buf);
    }

    fn render_popup(&mut self, area: Layout, buf: &mut Buffer) {
        let Some(popup) = self.popup_search.as_mut() else {
            return;
        };
        if popup.input.gained_focus() {
            self.state.focus.set(false);
        }

        let vert = TuiLayout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(12),
                Constraint::Percentage(76),
                Constraint::Percentage(12),
            ])
            .split(area.main_content);
        let hor = TuiLayout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(8),
                Constraint::Percentage(84),
                Constraint::Percentage(8),
            ])
            .split(vert[1]);
        let popup_area = hor[1];

        Clear.render(popup_area, buf);

        let sections = TuiLayout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(popup_area);
        let input_area = sections[0];
        let list_area = sections[1];
        let status_area = sections[2];

        let mut popup_title = "[Label Search] Regex".to_string();
        if popup.loading {
            popup_title.push_str(" | Searching");
        } else {
            popup_title.push_str(" | Enter: Search");
        }
        popup_title.push_str(if popup.case_insensitive {
            " | CI:on"
        } else {
            " | CI:off"
        });
        popup_title.push_str(" | a:Add Esc:Close");

        let mut input_block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&popup.input));
        if !popup.loading {
            input_block = input_block.title(popup_title);
        }

        let input = TextInput::new().block(input_block);
        input.render(input_area, buf, &mut popup.input);

        if popup.loading {
            let title_area = ratatui::layout::Rect {
                x: input_area.x + 1,
                y: input_area.y,
                width: 10,
                height: 1,
            };
            let throbber = Throbber::default()
                .label("Loading")
                .style(Style::default().fg(Color::Cyan))
                .throbber_set(BRAILLE_SIX_DOUBLE)
                .use_type(WhichUse::Spin);
            StatefulWidget::render(throbber, title_area, buf, &mut popup.throbber_state);
        }

        let list_block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&popup.list_state))
            .title("Matches");
        let list =
            rat_widget::list::List::<RowSelection>::new(popup.matches.iter().map(popup_list_item))
                .select_style(Style::default().bg(Color::Black))
                .focus_style(Style::default().bold().bg(Color::Black))
                .block(list_block);
        list.render(list_area, buf, &mut popup.list_state);

        if popup.matches.is_empty() && !popup.loading {
            let message = if let Some(err) = &popup.error {
                tracing::error!("Label search error: {err}");
                format!("Error: {err}")
            } else if popup.input.text().trim().is_empty() {
                "Type a regex query and press Enter to search.".to_string()
            } else {
                "No matches.".to_string()
            };
            Paragraph::new(message).render(list_area, buf);
        }

        let status = format!(
            "Scanned: {}  Matched: {}",
            popup.scanned_count, popup.matched_count
        );
        Paragraph::new(status).render(status_area, buf);
    }

    fn needs_footer(&self) -> bool {
        !matches!(self.mode, LabelEditMode::Idle)
    }

    fn expire_status(&mut self) {
        if let Some(status) = &self.status_message
            && status.at.elapsed() > STATUS_TTL
        {
            self.status_message = None;
        }
    }

    fn set_status(&mut self, message: impl Into<String>) {
        let message = message.into().replace('\n', " ");
        self.status_message = Some(StatusMessage {
            message,
            at: Instant::now(),
        });
    }

    fn set_mode(&mut self, mode: LabelEditMode) {
        self.mode = mode;
    }

    fn reset_selection(&mut self, previous_name: Option<String>) {
        if self.labels.is_empty() {
            self.state.clear_selection();
            return;
        }
        if let Some(name) = previous_name
            && let Some(idx) = self.labels.iter().position(|l| l.name == name)
        {
            self.state.select(Some(idx));
            return;
        }
        let _ = self.state.select(Some(0));
    }

    fn is_not_found(err: &OctoError) -> bool {
        matches!(
            err,
            OctoError::GitHub { source, .. } if source.status_code.as_u16() == 404
        )
    }

    fn normalize_label_name(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn normalize_color(input: &str) -> Result<String, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Ok(DEFAULT_COLOR.to_string());
        }
        let trimmed = trimmed.trim_start_matches('#');
        let is_hex = trimmed.len() == 6 && trimmed.chars().all(|c| c.is_ascii_hexdigit());
        if is_hex {
            Ok(trimmed.to_lowercase())
        } else {
            Err("Invalid color. Use 6 hex digits like eeddee.".to_string())
        }
    }

    fn open_popup_search(&mut self) {
        if self.popup_search.is_some() {
            return;
        }
        let input = TextInputState::new_focused();
        input.focus.set(true);
        self.state.focus.set(false);
        self.popup_search = Some(PopupLabelSearchState {
            input,
            list_state: ListState::default(),
            matches: Vec::new(),
            loading: false,
            case_insensitive: false,
            request_id: 0,
            scanned_count: 0,
            matched_count: 0,
            error: None,
            throbber_state: ThrobberState::default(),
        });
    }

    fn close_popup_search(&mut self) {
        self.popup_search = None;
    }

    fn build_popup_regex(query: &str, case_insensitive: bool) -> Result<regex::Regex, String> {
        RegexBuilder::new(query)
            .case_insensitive(case_insensitive)
            .build()
            .map_err(|err| err.to_string().replace('\n', " "))
    }

    fn append_popup_matches(&mut self, items: Vec<Label>) {
        let Some(popup) = self.popup_search.as_mut() else {
            return;
        };
        popup
            .matches
            .extend(items.into_iter().map(Into::<LabelListItem>::into));
        if popup.list_state.selected_checked().is_none() && !popup.matches.is_empty() {
            let _ = popup.list_state.select(Some(0));
        }
    }

    async fn start_popup_search(&mut self) {
        let Some(popup) = self.popup_search.as_mut() else {
            return;
        };
        if popup.loading {
            return;
        }

        let query = popup.input.text().trim().to_string();
        if query.is_empty() {
            popup.error = Some("Regex query required.".to_string());
            return;
        }
        let regex = match Self::build_popup_regex(&query, popup.case_insensitive) {
            Ok(regex) => regex,
            Err(message) => {
                popup.error = Some(message);
                return;
            }
        };

        self.label_search_request_seq = self.label_search_request_seq.saturating_add(1);
        let request_id = self.label_search_request_seq;
        popup.request_id = request_id;
        popup.loading = true;
        popup.error = None;
        popup.scanned_count = 0;
        popup.matched_count = 0;
        popup.matches.clear();
        popup.list_state.clear_selection();

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
                    .send(Action::LabelSearchError {
                        request_id,
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let crab = client.inner();
            let handler = crab.issues(owner, repo);

            let first = handler
                .list_labels_for_repo()
                .per_page(100u8)
                .page(1u32)
                .send()
                .await;

            let mut page = match first {
                Ok(page) => page,
                Err(err) => {
                    let _ = action_tx
                        .send(Action::LabelSearchError {
                            request_id,
                            message: err.to_string().replace('\n', " "),
                        })
                        .await;
                    return;
                }
            };

            let mut scanned = 0_u32;
            let mut matched = 0_u32;
            loop {
                let page_items = std::mem::take(&mut page.items);
                scanned = scanned.saturating_add(min(page_items.len(), u32::MAX as usize) as u32);
                let mut filtered = Vec::new();
                for label in page_items {
                    if regex.is_match(&label.name) {
                        matched = matched.saturating_add(1);
                        filtered.push(label);
                    }
                }
                if !filtered.is_empty() {
                    let _ = action_tx
                        .send(Action::LabelSearchPageAppend {
                            request_id,
                            items: filtered,
                            scanned,
                            matched,
                        })
                        .await;
                }

                if page.next.is_none() {
                    break;
                }
                let next_page = crab.get_page::<Label>(&page.next).await;
                match next_page {
                    Ok(Some(next_page)) => page = next_page,
                    Ok(None) => break,
                    Err(err) => {
                        let _ = action_tx
                            .send(Action::LabelSearchError {
                                request_id,
                                message: err.to_string().replace('\n', " "),
                            })
                            .await;
                        return;
                    }
                }
            }

            let _ = action_tx
                .send(Action::LabelSearchFinished {
                    request_id,
                    scanned,
                    matched,
                })
                .await;
        });
    }

    async fn apply_selected_popup_label(&mut self) {
        let Some(popup) = self.popup_search.as_mut() else {
            return;
        };
        let Some(selected) = popup.list_state.selected_checked() else {
            popup.error = Some("No matching label selected.".to_string());
            return;
        };
        let Some(label) = popup.matches.get(selected) else {
            popup.error = Some("No matching label selected.".to_string());
            return;
        };
        let name = label.name.clone();
        self.handle_add_submit(name).await;
        self.close_popup_search();
    }

    async fn handle_popup_event(&mut self, event: &crossterm::event::Event) -> bool {
        let Some(popup) = self.popup_search.as_mut() else {
            return false;
        };

        if matches!(event, ct_event!(keycode press Esc)) {
            self.close_popup_search();
            return true;
        }
        if matches!(
            event,
            ct_event!(key press CONTROL-'i') | ct_event!(key press ALT-'i')
        ) {
            popup.case_insensitive = !popup.case_insensitive;
            return true;
        }
        if matches!(event, ct_event!(keycode press Enter)) {
            self.start_popup_search().await;
            return true;
        }
        if matches!(event, ct_event!(key press CONTROL-'a')) {
            self.apply_selected_popup_label().await;
            return true;
        }
        if matches!(
            event,
            ct_event!(keycode press Up) | ct_event!(keycode press Down)
        ) {
            popup.list_state.handle(event, Regular);
            return true;
        }

        popup.input.handle(event, Regular);
        true
    }

    async fn handle_add_submit(&mut self, name: String) {
        let Some(issue_number) = self.current_issue_number else {
            self.set_status("No issue selected.");
            return;
        };
        if self.labels.iter().any(|l| l.name == name) {
            self.set_status("Label already applied.");
            return;
        }

        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.pending_status = Some(format!("Added: {name}"));

        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::LabelEditError {
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let handler = client.inner().issues(owner, repo);
            match handler.get_label(&name).await {
                Ok(_) => match handler
                    .add_labels(issue_number, slice::from_ref(&name))
                    .await
                {
                    Ok(labels) => {
                        let _ = action_tx
                            .send(Action::IssueLabelsUpdated {
                                number: issue_number,
                                labels,
                            })
                            .await;
                    }
                    Err(err) => {
                        let _ = action_tx
                            .send(Action::LabelEditError {
                                message: err.to_string(),
                            })
                            .await;
                    }
                },
                Err(err) => {
                    if LabelList::is_not_found(&err) {
                        let _ = action_tx
                            .send(Action::LabelMissing { name: name.clone() })
                            .await;
                    } else {
                        let _ = action_tx
                            .send(Action::LabelEditError {
                                message: err.to_string(),
                            })
                            .await;
                    }
                }
            }
        });
    }

    async fn handle_remove_selected(&mut self) {
        let Some(issue_number) = self.current_issue_number else {
            self.set_status("No issue selected.");
            return;
        };
        let Some(selected) = self.state.selected_checked() else {
            self.set_status("No label selected.");
            return;
        };
        let Some(label) = self.labels.get(selected) else {
            self.set_status("No label selected.");
            return;
        };
        let name = label.name.clone();

        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.pending_status = Some(format!("Removed: {name}"));

        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::LabelEditError {
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let handler = client.inner().issues(owner, repo);
            match handler.remove_label(issue_number, &name).await {
                Ok(labels) => {
                    let _ = action_tx
                        .send(Action::IssueLabelsUpdated {
                            number: issue_number,
                            labels,
                        })
                        .await;
                }
                Err(err) => {
                    error!("Failed to remove label: {err}");
                    let _ = action_tx
                        .send(Action::LabelEditError {
                            message: err.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    async fn handle_create_and_add(&mut self, name: String, color: String) {
        let Some(issue_number) = self.current_issue_number else {
            self.set_status("No issue selected.");
            return;
        };
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.pending_status = Some(format!("Added: {name}"));

        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::LabelEditError {
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let handler = client.inner().issues(owner, repo);
            match handler.create_label(&name, &color, "").await {
                Ok(_) => match handler
                    .add_labels(issue_number, slice::from_ref(&name))
                    .await
                {
                    Ok(labels) => {
                        let _ = action_tx
                            .send(Action::IssueLabelsUpdated {
                                number: issue_number,
                                labels,
                            })
                            .await;
                    }
                    Err(err) => {
                        let _ = action_tx
                            .send(Action::LabelEditError {
                                message: err.to_string(),
                            })
                            .await;
                    }
                },
                Err(err) => {
                    let _ = action_tx
                        .send(Action::LabelEditError {
                            message: err.to_string(),
                        })
                        .await;
                }
            }
        });
    }
}

#[async_trait(?Send)]
impl Component for LabelList {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }
    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        self.action_tx = Some(action_tx);
    }
    async fn handle_event(&mut self, event: Action) -> Result<(), AppError> {
        match event {
            Action::AppEvent(ref event) => {
                if self.screen == MainScreen::DetailsFullscreen {
                    return Ok(());
                }
                if self.handle_popup_event(event).await {
                    return Ok(());
                }

                enum SubmitAction {
                    Add(String),
                    Create { name: String, color: String },
                }

                let mut mode = std::mem::replace(&mut self.mode, LabelEditMode::Idle);
                let mut next_mode: Option<LabelEditMode> = None;
                let mut submit_action: Option<SubmitAction> = None;

                match &mut mode {
                    LabelEditMode::Idle => {
                        let mut handled = false;
                        if let crossterm::event::Event::Key(key) = event
                            && self.popup_search.is_none()
                        {
                            match key.code {
                                crossterm::event::KeyCode::Char('a') => {
                                    if self.state.is_focused() {
                                        let input = TextInputState::new_focused();
                                        next_mode = Some(LabelEditMode::Adding { input });
                                        handled = true;
                                    }
                                }
                                crossterm::event::KeyCode::Char('d') => {
                                    if self.state.is_focused() {
                                        self.handle_remove_selected().await;
                                        handled = true;
                                    }
                                }
                                crossterm::event::KeyCode::Char('f') => {
                                    if self.state.is_focused() {
                                        self.open_popup_search();
                                        handled = true;
                                    }
                                }
                                _ => {}
                            }
                        }
                        if !handled {
                            self.state.handle(event, Regular);
                        }
                    }
                    LabelEditMode::Adding { input } => {
                        let mut skip_input = false;
                        if let crossterm::event::Event::Key(key) = event {
                            match key.code {
                                crossterm::event::KeyCode::Enter => {
                                    if let Some(name) = Self::normalize_label_name(input.text()) {
                                        submit_action = Some(SubmitAction::Add(name));
                                        next_mode = Some(LabelEditMode::Idle);
                                    } else {
                                        self.set_status("Label name required.");
                                        skip_input = true;
                                    }
                                }
                                crossterm::event::KeyCode::Esc => {
                                    next_mode = Some(LabelEditMode::Idle);
                                }
                                _ => {}
                            }
                        }
                        if next_mode.is_none() && !skip_input {
                            input.handle(event, Regular);
                        }
                    }
                    LabelEditMode::ConfirmCreate { name } => {
                        if let crossterm::event::Event::Key(key) = event {
                            match key.code {
                                crossterm::event::KeyCode::Char('y')
                                | crossterm::event::KeyCode::Char('Y') => {
                                    let mut input = TextInputState::new_focused();
                                    input.set_text(DEFAULT_COLOR);
                                    next_mode = Some(LabelEditMode::CreateColor {
                                        name: name.clone(),
                                        input,
                                    });
                                }
                                crossterm::event::KeyCode::Char('n')
                                | crossterm::event::KeyCode::Char('N')
                                | crossterm::event::KeyCode::Esc => {
                                    self.pending_status = None;
                                    next_mode = Some(LabelEditMode::Idle);
                                }
                                _ => {}
                            }
                        }
                    }
                    LabelEditMode::CreateColor { name, input } => {
                        let mut skip_input = false;
                        if let crossterm::event::Event::Key(key) = event {
                            match key.code {
                                crossterm::event::KeyCode::Enter => {
                                    match Self::normalize_color(input.text()) {
                                        Ok(color) => {
                                            submit_action = Some(SubmitAction::Create {
                                                name: name.clone(),
                                                color,
                                            });
                                            next_mode = Some(LabelEditMode::Idle);
                                        }
                                        Err(message) => {
                                            self.set_status(message);
                                            skip_input = true;
                                        }
                                    }
                                }
                                crossterm::event::KeyCode::Esc => {
                                    next_mode = Some(LabelEditMode::Idle);
                                }
                                _ => {}
                            }
                        }
                        if next_mode.is_none() && !skip_input {
                            input.handle(event, Regular);
                        }
                    }
                }

                self.mode = next_mode.unwrap_or(mode);

                if let Some(action) = submit_action {
                    match action {
                        SubmitAction::Add(name) => self.handle_add_submit(name).await,
                        SubmitAction::Create { name, color } => {
                            self.handle_create_and_add(name, color).await
                        }
                    }
                }
            }
            Action::SelectedIssue { number, labels } => {
                let prev = self
                    .state
                    .selected_checked()
                    .and_then(|idx| self.labels.get(idx).map(|label| label.name.clone()));
                self.labels = labels
                    .into_iter()
                    .map(Into::<LabelListItem>::into)
                    .collect();
                self.current_issue_number = Some(number);
                self.reset_selection(prev);
                self.pending_status = None;
                self.status_message = None;
                self.set_mode(LabelEditMode::Idle);
                self.close_popup_search();
            }
            Action::IssueLabelsUpdated { number, labels } => {
                if Some(number) == self.current_issue_number {
                    let prev = self
                        .state
                        .selected_checked()
                        .and_then(|idx| self.labels.get(idx).map(|label| label.name.clone()));
                    self.labels = labels
                        .into_iter()
                        .map(Into::<LabelListItem>::into)
                        .collect();
                    self.reset_selection(prev);
                    let status = self
                        .pending_status
                        .take()
                        .unwrap_or_else(|| "Labels updated.".to_string());
                    self.set_status(status);
                    self.set_mode(LabelEditMode::Idle);
                }
            }
            Action::LabelSearchPageAppend {
                request_id,
                items,
                scanned,
                matched,
            } => {
                if let Some(popup) = self.popup_search.as_mut() {
                    if popup.request_id != request_id {
                        return Ok(());
                    }
                    popup.scanned_count = scanned;
                    popup.matched_count = matched;
                    popup.error = None;
                } else {
                    return Ok(());
                }
                self.append_popup_matches(items);
            }
            Action::LabelSearchFinished {
                request_id,
                scanned,
                matched,
            } => {
                if let Some(popup) = self.popup_search.as_mut() {
                    if popup.request_id != request_id {
                        return Ok(());
                    }
                    popup.loading = false;
                    popup.scanned_count = scanned;
                    popup.matched_count = matched;
                    popup.error = None;
                }
            }
            Action::LabelSearchError {
                request_id,
                message,
            } => {
                if let Some(popup) = self.popup_search.as_mut() {
                    if popup.request_id != request_id {
                        return Ok(());
                    }
                    popup.loading = false;
                    popup.error = Some(message);
                }
            }
            Action::LabelMissing { name } => {
                self.set_status("Label not found.");
                self.set_mode(LabelEditMode::ConfirmCreate { name });
            }
            Action::LabelEditError { message } => {
                self.pending_status = None;
                self.set_status(format!("Error: {message}"));
                self.set_mode(LabelEditMode::Idle);
            }
            Action::Tick => {
                if let Some(popup) = self.popup_search.as_mut()
                    && popup.loading
                {
                    popup.throbber_state.calc_next();
                }
            }
            Action::ChangeIssueScreen(screen) => {
                self.screen = screen;
                if screen == MainScreen::DetailsFullscreen {
                    self.mode = LabelEditMode::Idle;
                    self.popup_search = None;
                    self.status_message = None;
                    self.pending_status = None;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn should_render(&self) -> bool {
        self.screen != MainScreen::DetailsFullscreen
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        if let Some(popup) = &self.popup_search {
            return popup.input.screen_cursor();
        }
        match &self.mode {
            LabelEditMode::Adding { input } => input.screen_cursor(),
            LabelEditMode::CreateColor { input, .. } => input.screen_cursor(),
            _ => None,
        }
    }

    fn is_animating(&self) -> bool {
        self.status_message.is_some()
            || self
                .popup_search
                .as_ref()
                .is_some_and(|popup| popup.loading)
    }
    fn set_index(&mut self, index: usize) {
        self.index = index;
    }

    fn set_global_help(&self) {
        if let Some(action_tx) = &self.action_tx {
            let _ = action_tx.try_send(Action::SetHelp(HELP));
        }
    }

    fn capture_focus_event(&self, _event: &crossterm::event::Event) -> bool {
        self.popup_search.is_some()
            || matches!(
                self.mode,
                LabelEditMode::Adding { .. } | LabelEditMode::CreateColor { .. }
            )
    }
}
impl HasFocus for LabelList {
    fn build(&self, builder: &mut rat_widget::focus::FocusBuilder) {
        let tag = builder.start(self);
        builder.widget(&self.state);
        if let Some(popup) = &self.popup_search {
            builder.widget(&popup.input);
            builder.widget(&popup.list_state);
        }
        builder.end(tag);
    }
    fn area(&self) -> ratatui::layout::Rect {
        self.state.area()
    }
    fn focus(&self) -> rat_widget::focus::FocusFlag {
        self.state.focus()
    }
}
