use std::{
    slice,
    str::FromStr,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use octocrab::Error as OctoError;
use octocrab::models::Label;
use rat_cursor::HasScreenCursor;
use rat_widget::{
    event::{HandleEvent, Regular},
    focus::HasFocus,
    list::{ListState, selection::RowSelection},
    text_input::{TextInput, TextInputState},
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout as TuiLayout},
    style::{Color, Style, Stylize},
    widgets::{Block, ListItem, Paragraph, StatefulWidget, Widget},
};
use ratatui_macros::{line, span};

use crate::{
    app::GITHUB_CLIENT,
    ui::{
        Action, AppState, COLOR_PROFILE, components::Component, layout::Layout,
        utils::get_border_style,
    },
};

const MARKER: &str = ratatui::symbols::marker::DOT;
const STATUS_TTL: Duration = Duration::from_secs(3);
const DEFAULT_COLOR: &str = "ededed";

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
        let mut c = Color::from_str(&format!("#{}", rgb)).unwrap();
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
            format!("Labels (a:add d:remove) | {}", status.message)
        } else {
            "Labels (a:add d:remove)".to_string()
        };
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&self.state))
            .title(title);
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
                    Paragraph::new(prompt).render(area, buf);
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
    async fn handle_event(&mut self, event: Action) {
        match event {
            Action::AppEvent(ref event) => {
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
                        if let crossterm::event::Event::Key(key) = event {
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
            Action::LabelMissing { name } => {
                self.set_status("Label not found.");
                self.set_mode(LabelEditMode::ConfirmCreate { name });
            }
            Action::LabelEditError { message } => {
                self.pending_status = None;
                self.set_status(format!("Error: {message}"));
                self.set_mode(LabelEditMode::Idle);
            }
            _ => {}
        }
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        match &self.mode {
            LabelEditMode::Adding { input } => input.screen_cursor(),
            LabelEditMode::CreateColor { input, .. } => input.screen_cursor(),
            _ => None,
        }
    }

    fn is_animating(&self) -> bool {
        self.status_message.is_some()
    }
}
impl HasFocus for LabelList {
    fn build(&self, builder: &mut rat_widget::focus::FocusBuilder) {
        let tag = builder.start(self);
        builder.widget(&self.state);
        builder.end(tag);
    }
    fn area(&self) -> ratatui::layout::Rect {
        self.state.area()
    }
    fn focus(&self) -> rat_widget::focus::FocusFlag {
        self.state.focus()
    }
}
