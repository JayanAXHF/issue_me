use async_trait::async_trait;
use crossterm::event;
use octocrab::models::issues::Issue;
use rat_cursor::HasScreenCursor;
use rat_widget::{
    event::{HandleEvent, TextOutcome, ct_event},
    focus::{FocusBuilder, FocusFlag, HasFocus, Navigation},
    paragraph::{Paragraph, ParagraphState},
    text_input::{TextInput, TextInputState},
    textarea::{TextArea, TextAreaState, TextWrap},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, StatefulWidget},
};
use ratatui_macros::vertical;
use throbber_widgets_tui::{BRAILLE_SIX_DOUBLE, Throbber, ThrobberState, WhichUse};

use crate::{
    app::GITHUB_CLIENT,
    errors::AppError,
    ui::{
        Action, AppState,
        components::{
            Component,
            help::HelpElementKind,
            issue_conversation::{IssueConversationSeed, render_markdown_lines},
            issue_detail::IssuePreviewSeed,
            issue_list::MainScreen,
        },
        layout::Layout,
        toast_action,
        utils::get_border_style,
    },
};
use anyhow::anyhow;
use ratatui_toaster::ToastType;

pub const HELP: &[HelpElementKind] = &[
    crate::help_text!("Issue Create Help"),
    crate::help_keybind!("n", "open new issue composer (from issue list)"),
    crate::help_keybind!("Tab / Shift+Tab", "switch fields"),
    crate::help_keybind!("Ctrl+P", "toggle body input and markdown preview"),
    crate::help_keybind!("Ctrl+Enter / Alt+Enter", "create issue"),
    crate::help_keybind!("Esc", "return to issue list"),
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum InputMode {
    #[default]
    Input,
    Preview,
}

impl InputMode {
    fn toggle(&mut self) {
        *self = match self {
            Self::Input => Self::Preview,
            Self::Preview => Self::Input,
        };
    }
}

pub struct IssueCreate {
    action_tx: Option<tokio::sync::mpsc::Sender<Action>>,
    owner: String,
    repo: String,
    screen: MainScreen,
    focus: FocusFlag,
    area: Rect,
    index: usize,
    title_state: TextInputState,
    labels_state: TextInputState,
    assignees_state: TextInputState,
    body_state: TextAreaState,
    preview_state: ParagraphState,
    mode: InputMode,
    creating: bool,
    create_throbber_state: ThrobberState,
    error: Option<String>,
    preview_cache_input: String,
    preview_cache_width: usize,
    preview_cache: Vec<ratatui::text::Line<'static>>,
}

impl IssueCreate {
    pub fn new(AppState { owner, repo, .. }: AppState) -> Self {
        Self {
            action_tx: None,
            owner,
            repo,
            screen: MainScreen::List,
            focus: FocusFlag::new().with_name("issue_create"),
            area: Rect::default(),
            index: 0,
            title_state: TextInputState::default(),
            labels_state: TextInputState::default(),
            assignees_state: TextInputState::default(),
            body_state: TextAreaState::new(),
            preview_state: ParagraphState::default(),
            mode: InputMode::default(),
            creating: false,
            create_throbber_state: ThrobberState::default(),
            error: None,
            preview_cache_input: String::new(),
            preview_cache_width: 0,
            preview_cache: Vec::new(),
        }
    }

    fn reset_form(&mut self) {
        self.title_state.set_text("");
        self.labels_state.set_text("");
        self.assignees_state.set_text("");
        self.body_state.set_text("");
        self.error = None;
        self.mode = InputMode::Input;
        self.preview_state.focus.set(false);
        self.title_state.focus.set(true);
        self.labels_state.focus.set(false);
        self.assignees_state.focus.set(false);
        self.body_state.focus.set(false);
        self.preview_cache_input.clear();
        self.preview_cache.clear();
        self.preview_cache_width = 0;
    }

    fn parse_csv(input: &str) -> Option<Vec<String>> {
        let values = input
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if values.is_empty() {
            None
        } else {
            Some(values)
        }
    }

    fn body_preview_lines(&mut self, width: usize) -> &[ratatui::text::Line<'static>] {
        let body = self.body_state.text();
        if self.preview_cache_width != width || self.preview_cache_input != body {
            self.preview_cache_width = width;
            self.preview_cache_input.clear();
            self.preview_cache_input.push_str(&body);
            self.preview_cache = render_markdown_lines(&self.preview_cache_input, width, 2);
        }
        self.preview_cache.as_slice()
    }

    async fn submit(&mut self) {
        if self.creating {
            return;
        }
        let title = self.title_state.text().trim().to_string();
        if title.is_empty() {
            self.error = Some("Title cannot be empty.".to_string());
            return;
        }

        let body = self.body_state.text().trim().to_string();
        let labels = Self::parse_csv(self.labels_state.text());
        let assignees = Self::parse_csv(self.assignees_state.text());

        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.creating = true;
        self.error = None;

        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::IssueCreateError {
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let issues = client.inner().issues(owner, repo);
            let mut create = issues.create(title);
            if !body.is_empty() {
                create = create.body(body);
            }
            if let Some(labels) = labels {
                create = create.labels(labels);
            }
            if let Some(assignees) = assignees {
                create = create.assignees(assignees);
            }

            match create.send().await {
                Ok(issue) => {
                    let _ = action_tx
                        .send(Action::IssueCreateSuccess {
                            issue: Box::new(issue),
                        })
                        .await;
                    let _ = action_tx
                        .send(toast_action(
                            "Issue Created Successfully!",
                            ToastType::Success,
                        ))
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::IssueCreateError {
                            message: err.to_string().replace('\n', " "),
                        })
                        .await;
                    let _ = action_tx
                        .send(toast_action("Failed to create issue.", ToastType::Error))
                        .await;
                }
            }
        });
    }

    async fn handle_create_success(&mut self, issue: Issue) {
        self.creating = false;
        self.error = None;
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let number = issue.number;
        let labels = issue.labels.clone();
        let preview_seed = IssuePreviewSeed::from_issue(&issue);
        let conversation_seed = IssueConversationSeed::from_issue(&issue);
        self.reset_form();
        let _ = action_tx
            .send(Action::SelectedIssue { number, labels })
            .await;
        let _ = action_tx
            .send(Action::SelectedIssuePreview { seed: preview_seed })
            .await;
        let _ = action_tx
            .send(Action::EnterIssueDetails {
                seed: conversation_seed,
            })
            .await;
        let _ = action_tx
            .send(Action::ChangeIssueScreen(MainScreen::Details))
            .await;
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.area = area.main_content;
        let [title_area, labels_area, assignees_area, body_area] = vertical![==3, ==3, ==3, *=1]
            .areas(
                area.main_content
                    .union(area.text_search.union(area.label_search)),
            );

        let title_input = TextInput::new().block(
            Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(get_border_style(&self.title_state))
                .title(format!("[{}] Title", self.index)),
        );
        title_input.render(title_area, buf, &mut self.title_state);

        let labels_input = TextInput::new().block(
            Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(get_border_style(&self.labels_state))
                .title("Labels (comma-separated)"),
        );
        labels_input.render(labels_area, buf, &mut self.labels_state);

        let assignees_input = TextInput::new().block(
            Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(get_border_style(&self.assignees_state))
                .title("Assignees (comma-separated)"),
        );
        assignees_input.render(assignees_area, buf, &mut self.assignees_state);

        match self.mode {
            InputMode::Input => {
                let mut title = "Body (Ctrl+P: Preview | Ctrl+Enter: Create)".to_string();
                if let Some(err) = &self.error {
                    title.push_str(" | ");
                    title.push_str(err);
                }
                let mut block = Block::bordered()
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(get_border_style(&self.body_state));
                if !self.creating {
                    block = block.title(title);
                }
                let textarea = TextArea::new().block(block).text_wrap(TextWrap::Word(4));
                textarea.render(body_area, buf, &mut self.body_state);
            }
            InputMode::Preview => {
                let mut title = "Preview (Ctrl+P: Edit | Ctrl+Enter: Create)".to_string();
                if let Some(err) = &self.error {
                    title.push_str(" | ");
                    title.push_str(err);
                }
                let preview_width = body_area.width.saturating_sub(4).max(10) as usize;
                let lines = self.body_preview_lines(preview_width).to_vec();
                let preview = Paragraph::new(lines)
                    .block(
                        Block::bordered()
                            .border_type(ratatui::widgets::BorderType::Rounded)
                            .border_style(get_border_style(&self.preview_state))
                            .title(title),
                    )
                    .focus_style(Style::default())
                    .hide_focus(true)
                    .wrap(ratatui::widgets::Wrap { trim: false });
                preview.render(body_area, buf, &mut self.preview_state);
            }
        }

        if self.creating {
            let title_area = Rect {
                x: body_area.x + 1,
                y: body_area.y,
                width: 10,
                height: 1,
            };
            let throbber = Throbber::default()
                .label("Creating")
                .style(Style::new().fg(Color::Cyan))
                .throbber_set(BRAILLE_SIX_DOUBLE)
                .use_type(WhichUse::Spin);
            StatefulWidget::render(throbber, title_area, buf, &mut self.create_throbber_state);
        }
    }
}

#[async_trait(?Send)]
impl Component for IssueCreate {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }

    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        self.action_tx = Some(action_tx);
    }

    async fn handle_event(&mut self, event: Action) -> Result<(), AppError> {
        match event {
            Action::AppEvent(ref event) => {
                if self.screen != MainScreen::CreateIssue {
                    return Ok(());
                }
                match event {
                    ct_event!(keycode press Esc) => {
                        if let Some(action_tx) = self.action_tx.clone() {
                            let _ = action_tx
                                .send(Action::ChangeIssueScreen(MainScreen::List))
                                .await;
                        }
                        return Ok(());
                    }
                    ct_event!(key press CONTROL-'p') => {
                        self.mode.toggle();
                        match self.mode {
                            InputMode::Input => {
                                self.preview_state.focus.set(false);
                                self.body_state.focus.set(true);
                            }
                            InputMode::Preview => {
                                self.body_state.focus.set(false);
                                self.preview_state.focus.set(true);
                            }
                        }
                        return Ok(());
                    }
                    ct_event!(keycode press CONTROL-Enter) | ct_event!(keycode press ALT-Enter) => {
                        self.submit().await;
                        return Ok(());
                    }
                    ct_event!(keycode press Tab) | ct_event!(keycode press SHIFT-Tab)
                        if self.body_state.is_focused() =>
                    {
                        if let Some(action_tx) = self.action_tx.clone() {
                            let _ = action_tx.send(Action::ForceFocusChange).await;
                        }
                        return Ok(());
                    }
                    _ => {}
                }

                self.title_state.handle(event, rat_widget::event::Regular);
                self.labels_state.handle(event, rat_widget::event::Regular);
                self.assignees_state
                    .handle(event, rat_widget::event::Regular);

                if matches!(
                    event,
                    ct_event!(keycode press Up)
                        | ct_event!(keycode press Down)
                        | ct_event!(keycode press Left)
                        | ct_event!(keycode press Right)
                ) {
                    let action_tx = self.action_tx.as_ref().ok_or_else(|| {
                        AppError::Other(anyhow!("issue create action channel unavailable"))
                    })?;
                    action_tx.send(Action::ForceRender).await?;
                }
                match self.mode {
                    InputMode::Input => {
                        if let event::Event::Key(key) = event
                            && key.code == event::KeyCode::Tab
                        {
                            return Ok(());
                        }
                        if let event::Event::Paste(pasted_stuff) = event {
                            self.body_state.insert_str(pasted_stuff);
                        }
                        let o = self.body_state.handle(event, rat_widget::event::Regular);
                        if o == TextOutcome::TextChanged {
                            let action_tx = self.action_tx.as_ref().ok_or_else(|| {
                                AppError::Other(anyhow!("issue create action channel unavailable"))
                            })?;
                            action_tx.send(Action::ForceRender).await?;
                        }
                    }
                    InputMode::Preview => {
                        self.preview_state.handle(event, rat_widget::event::Regular);
                    }
                }
            }
            Action::Tick => {
                if self.creating {
                    self.create_throbber_state.calc_next();
                }
            }
            Action::EnterIssueCreate => {
                self.screen = MainScreen::CreateIssue;
                self.reset_form();
            }
            Action::IssueCreateSuccess { issue } => {
                if self.screen == MainScreen::CreateIssue {
                    self.handle_create_success(*issue).await;
                }
            }
            Action::IssueCreateError { message } => {
                self.creating = false;
                if self.screen == MainScreen::CreateIssue {
                    self.error = Some(message);
                }
            }
            Action::ChangeIssueScreen(screen) => {
                self.screen = screen;
                if screen != MainScreen::CreateIssue {
                    self.title_state.focus.set(false);
                    self.labels_state.focus.set(false);
                    self.assignees_state.focus.set(false);
                    self.body_state.focus.set(false);
                    self.preview_state.focus.set(false);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.title_state
            .screen_cursor()
            .or_else(|| self.labels_state.screen_cursor())
            .or_else(|| self.assignees_state.screen_cursor())
            .or_else(|| self.body_state.screen_cursor())
    }

    fn should_render(&self) -> bool {
        self.screen == MainScreen::CreateIssue
    }

    fn is_animating(&self) -> bool {
        self.screen == MainScreen::CreateIssue && self.creating
    }

    fn capture_focus_event(&self, event: &event::Event) -> bool {
        if self.screen != MainScreen::CreateIssue {
            return false;
        }
        if !(self.title_state.is_focused()
            || self.labels_state.is_focused()
            || self.assignees_state.is_focused()
            || self.body_state.is_focused())
        {
            return false;
        }
        if self.body_state.is_focused()
            && !matches!(
                event,
                ct_event!(keycode press Tab) | ct_event!(keycode press SHIFT-Tab)
            )
        {
            return true;
        }
        match event {
            event::Event::Key(key) => matches!(
                key.code,
                event::KeyCode::Char('q') | event::KeyCode::Tab | event::KeyCode::BackTab
            ),
            _ => false,
        }
    }

    fn set_index(&mut self, index: usize) {
        self.index = index;
    }

    fn set_global_help(&self) {
        if let Some(action_tx) = &self.action_tx {
            let _ = action_tx.try_send(Action::SetHelp(HELP));
        }
    }
}

impl HasFocus for IssueCreate {
    fn build(&self, builder: &mut FocusBuilder) {
        let tag = builder.start(self);
        builder.widget(&self.title_state);
        builder.widget(&self.labels_state);
        builder.widget(&self.assignees_state);
        match self.mode {
            InputMode::Input => builder.widget(&self.body_state),
            InputMode::Preview => builder.widget(&self.preview_state),
        };
        builder.end(tag);
    }

    fn focus(&self) -> FocusFlag {
        self.focus.clone()
    }

    fn area(&self) -> Rect {
        self.area
    }

    fn navigable(&self) -> Navigation {
        if self.screen == MainScreen::CreateIssue {
            Navigation::Regular
        } else {
            Navigation::None
        }
    }
}
