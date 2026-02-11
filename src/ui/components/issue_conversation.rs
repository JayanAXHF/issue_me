use async_trait::async_trait;
use crossterm::event;
use octocrab::models::issues::Comment as ApiComment;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use rat_cursor::HasScreenCursor;
use rat_widget::{
    event::{HandleEvent, TextOutcome, ct_event},
    focus::{FocusBuilder, FocusFlag, HasFocus, Navigation},
    list::{ListState, selection::RowSelection},
    paragraph::{Paragraph, ParagraphState},
    textarea::{TextArea, TextAreaState, TextWrap},
};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout as TuiLayout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, ListItem, StatefulWidget},
};
use ratatui_macros::{horizontal, line, vertical};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use textwrap::core::display_width;
use throbber_widgets_tui::{BRAILLE_SIX_DOUBLE, Throbber, ThrobberState, WhichUse};
use tracing::info;

use crate::{
    app::GITHUB_CLIENT,
    ui::{
        Action,
        components::{Component, issue_list::MainScreen},
        layout::Layout,
        utils::get_border_style,
    },
};

#[derive(Debug, Clone)]
pub struct IssueConversationSeed {
    pub number: u64,
    pub author: Arc<str>,
    pub created_at: Arc<str>,
    pub body: Option<Arc<str>>,
}

impl IssueConversationSeed {
    pub fn from_issue(issue: &octocrab::models::issues::Issue) -> Self {
        Self {
            number: issue.number,
            author: Arc::<str>::from(issue.user.login.as_str()),
            created_at: Arc::<str>::from(issue.created_at.format("%Y-%m-%d %H:%M").to_string()),
            body: issue.body.as_ref().map(|b| Arc::<str>::from(b.as_str())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommentView {
    pub id: u64,
    pub author: Arc<str>,
    pub created_at: Arc<str>,
    pub body: Arc<str>,
}

impl CommentView {
    pub fn from_api(comment: ApiComment) -> Self {
        let body = comment.body.unwrap_or_default();
        Self {
            id: comment.id.0,
            author: Arc::<str>::from(comment.user.login.as_str()),
            created_at: Arc::<str>::from(comment.created_at.format("%Y-%m-%d %H:%M").to_string()),
            body: Arc::<str>::from(body),
        }
    }
}

pub struct IssueConversation {
    action_tx: Option<tokio::sync::mpsc::Sender<Action>>,
    current: Option<IssueConversationSeed>,
    cache_number: Option<u64>,
    cache_comments: Vec<CommentView>,
    markdown_cache: HashMap<u64, Vec<Line<'static>>>,
    body_cache: Option<Vec<Line<'static>>>,
    body_cache_number: Option<u64>,
    markdown_width: usize,
    loading: HashSet<u64>,
    posting: bool,
    error: Option<String>,
    post_error: Option<String>,
    owner: String,
    repo: String,
    current_user: String,
    list_state: ListState<RowSelection>,
    message_keys: Vec<MessageKey>,
    input_state: TextAreaState,
    throbber_state: ThrobberState,
    post_throbber_state: ThrobberState,
    screen: MainScreen,
    focus: FocusFlag,
    area: Rect,
    textbox_state: InputState,
    paragraph_state: ParagraphState,
    body_paragraph_state: ParagraphState,
    index: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum InputState {
    #[default]
    Input,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageKey {
    IssueBody(u64),
    Comment(u64),
}

impl InputState {
    fn toggle(&mut self) {
        *self = match self {
            InputState::Input => InputState::Preview,
            InputState::Preview => InputState::Input,
        };
    }
}

impl IssueConversation {
    pub fn new(app_state: crate::ui::AppState) -> Self {
        Self {
            action_tx: None,
            current: None,
            cache_number: None,
            cache_comments: Vec::new(),
            markdown_cache: HashMap::new(),
            paragraph_state: Default::default(),
            body_cache: None,
            body_cache_number: None,
            markdown_width: 0,
            loading: HashSet::new(),
            posting: false,
            error: None,
            post_error: None,
            owner: app_state.owner,
            repo: app_state.repo,
            current_user: app_state.current_user,
            list_state: ListState::default(),
            message_keys: Vec::new(),
            input_state: TextAreaState::new(),
            textbox_state: InputState::default(),
            throbber_state: ThrobberState::default(),
            post_throbber_state: ThrobberState::default(),
            screen: MainScreen::default(),
            focus: FocusFlag::new().with_name("issue_conversation"),
            area: Rect::default(),
            body_paragraph_state: ParagraphState::default(),
            index: 0,
        }
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.area = area.main_content;
        let areas = vertical![*=1, ==5].split(area.main_content);
        let content_area = areas[0];
        let input_area = areas[1];
        let content_split = horizontal![*=1, *=1].split(content_area);
        let list_area = content_split[0];
        let body_area = content_split[1];

        let items = self.build_items(list_area, body_area);
        info!("Rendering {} comments", items.len());
        let mut list_block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&self.list_state));

        if !self.is_loading_current() {
            list_block = list_block.title(format!("[{}] Conversation", self.index));
        }

        let list = rat_widget::list::List::<RowSelection>::new(items)
            .block(list_block)
            .style(Style::default())
            .focus_style(Style::default().bold().reversed())
            .select_style(Style::default().add_modifier(Modifier::BOLD));
        list.render(list_area, buf, &mut self.list_state);
        self.render_body(body_area, buf);
        if self.is_loading_current() {
            let title_area = Rect {
                x: list_area.x + 1,
                y: list_area.y,
                width: 10,
                height: 1,
            };
            let throbber = Throbber::default()
                .label("Loading")
                .style(Style::new().fg(Color::Cyan))
                .throbber_set(BRAILLE_SIX_DOUBLE)
                .use_type(WhichUse::Spin);
            StatefulWidget::render(throbber, title_area, buf, &mut self.throbber_state);
        }

        match self.textbox_state {
            InputState::Input => {
                let input_title = if let Some(err) = &self.post_error {
                    format!("Comment (Ctrl+Enter to send) | {err}")
                } else {
                    "Comment (Ctrl+Enter to send)".to_string()
                };
                let mut input_block = Block::bordered()
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(get_border_style(&self.input_state));
                if !self.posting {
                    input_block = input_block.title(input_title);
                }
                let input_widget = TextArea::new()
                    .block(input_block)
                    .text_wrap(TextWrap::Word(4));
                input_widget.render(input_area, buf, &mut self.input_state);
            }
            InputState::Preview => {
                let rendered =
                    render_markdown_lines(&self.input_state.text(), self.markdown_width, 2);
                let para = Paragraph::new(rendered)
                    .block(
                        Block::bordered()
                            .border_type(ratatui::widgets::BorderType::Rounded)
                            .border_style(get_border_style(&self.paragraph_state))
                            .title("Preview"),
                    )
                    .focus_style(Style::default())
                    .hide_focus(true)
                    .wrap(ratatui::widgets::Wrap { trim: true });

                para.render(input_area, buf, &mut self.paragraph_state);
            }
        }

        if self.posting {
            let title_area = Rect {
                x: input_area.x + 1,
                y: input_area.y,
                width: 10,
                height: 1,
            };
            let throbber = Throbber::default()
                .label("Sending")
                .style(Style::new().fg(Color::Cyan))
                .throbber_set(BRAILLE_SIX_DOUBLE)
                .use_type(WhichUse::Spin);
            StatefulWidget::render(throbber, title_area, buf, &mut self.post_throbber_state);
        }
    }

    fn build_items(&mut self, list_area: Rect, body_area: Rect) -> Vec<ListItem<'static>> {
        let mut items = Vec::new();
        let width = body_area.width.saturating_sub(4).max(10) as usize;
        let preview_width = list_area.width.saturating_sub(12).max(8) as usize;
        self.message_keys.clear();

        if self.markdown_width != width {
            self.markdown_width = width;
            self.markdown_cache.clear();
            self.body_cache = None;
            self.body_cache_number = None;
        }

        if let Some(err) = &self.error {
            items.push(ListItem::new(line![Span::styled(
                err.clone(),
                Style::new().fg(Color::Red)
            )]));
        }

        let Some(seed) = &self.current else {
            items.push(ListItem::new(line![Span::styled(
                "Press Enter on an issue to view the conversation.".to_string(),
                Style::new().dim()
            )]));
            self.list_state.clear_selection();
            return items;
        };

        if let Some(body) = seed
            .body
            .as_ref()
            .map(|b| b.as_ref())
            .filter(|b| !b.trim().is_empty())
        {
            if self.body_cache_number != Some(seed.number) {
                self.body_cache_number = Some(seed.number);
                self.body_cache = None;
            }
            let body_lines = self
                .body_cache
                .get_or_insert_with(|| render_markdown_lines(body, width, 2));
            items.push(build_comment_preview_item(
                seed.author.as_ref(),
                seed.created_at.as_ref(),
                body_lines,
                preview_width,
                seed.author.as_ref() == self.current_user,
            ));
            self.message_keys.push(MessageKey::IssueBody(seed.number));
        }

        if self.cache_number == Some(seed.number) {
            info!(
                "Rendering {} comments for #{}",
                self.cache_comments.len(),
                seed.number
            );
            for comment in &self.cache_comments {
                let body_lines = self
                    .markdown_cache
                    .entry(comment.id)
                    .or_insert_with(|| render_markdown_lines(comment.body.as_ref(), width, 2));
                items.push(build_comment_preview_item(
                    comment.author.as_ref(),
                    comment.created_at.as_ref(),
                    body_lines,
                    preview_width,
                    comment.author.as_ref() == self.current_user,
                ));
                self.message_keys.push(MessageKey::Comment(comment.id));
            }
        }

        if items.is_empty() {
            self.list_state.clear_selection();
        } else {
            let selected = self.list_state.selected_checked().unwrap_or(0);
            let clamped = selected.min(items.len() - 1);
            let _ = self.list_state.select(Some(clamped));
        }

        items
    }

    fn render_body(&mut self, body_area: Rect, buf: &mut Buffer) {
        let body_lines: Vec<Line<'static>> = self
            .selected_body_lines()
            .map(|v| v.to_vec())
            .unwrap_or_else(|| {
                vec![Line::from(vec![Span::styled(
                    "Select a message to view full content.".to_string(),
                    Style::new().dim(),
                )])]
            });

        let body = Paragraph::new(body_lines)
            .block(
                Block::bordered()
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .border_style(get_border_style(&self.body_paragraph_state))
                    .title("Message Body (PageUp/PageDown/Home/End)"),
            )
            .focus_style(Style::default())
            .hide_focus(true)
            .wrap(ratatui::widgets::Wrap { trim: false });

        body.render(body_area, buf, &mut self.body_paragraph_state);
    }

    fn selected_body_lines(&self) -> Option<&[Line<'static>]> {
        let selected = self.list_state.selected_checked()?;
        let key = self.message_keys.get(selected)?;
        match key {
            MessageKey::IssueBody(number) => {
                if self.body_cache_number == Some(*number) {
                    self.body_cache.as_deref()
                } else {
                    None
                }
            }
            MessageKey::Comment(id) => self.markdown_cache.get(id).map(Vec::as_slice),
        }
    }

    fn is_loading_current(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|seed| self.loading.contains(&seed.number))
    }

    async fn fetch_comments(&mut self, number: u64) {
        if self.loading.contains(&number) {
            return;
        }
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.loading.insert(number);
        self.error = None;

        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::IssueCommentsError {
                        number,
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let handler = client.inner().issues(owner, repo);
            let page = handler
                .list_comments(number)
                .per_page(100u8)
                .page(1u32)
                .send()
                .await;

            match page {
                Ok(mut p) => {
                    let comments: Vec<CommentView> = std::mem::take(&mut p.items)
                        .into_iter()
                        .map(CommentView::from_api)
                        .collect();
                    info!("Loaded {} comments for issue {}", comments.len(), number);
                    let _ = action_tx
                        .send(Action::IssueCommentsLoaded { number, comments })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::IssueCommentsError {
                            number,
                            message: err.to_string().replace('\n', " "),
                        })
                        .await;
                }
            }
        });
    }

    async fn send_comment(&mut self, number: u64, body: String) {
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        self.posting = true;
        self.post_error = None;

        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::IssueCommentPostError {
                        number,
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let handler = client.inner().issues(owner, repo);
            match handler.create_comment(number, body).await {
                Ok(comment) => {
                    let _ = action_tx
                        .send(Action::IssueCommentPosted {
                            number,
                            comment: CommentView::from_api(comment),
                        })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::IssueCommentPostError {
                            number,
                            message: err.to_string().replace('\n', " "),
                        })
                        .await;
                }
            }
        });
    }
}

#[async_trait(?Send)]
impl Component for IssueConversation {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }

    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        self.action_tx = Some(action_tx);
    }

    async fn handle_event(&mut self, event: Action) {
        match event {
            Action::AppEvent(ref event) => {
                if self.screen != MainScreen::Details {
                    return;
                }

                match event {
                    ct_event!(keycode press Tab) | ct_event!(keycode press SHIFT-Tab)
                        if self.input_state.is_focused() =>
                    {
                        self.action_tx
                            .as_ref()
                            .unwrap()
                            .send(Action::ForceFocusChange)
                            .await
                            .unwrap();
                    }
                    ct_event!(keycode press Esc) if self.body_paragraph_state.is_focused() => self
                        .action_tx
                        .as_ref()
                        .unwrap()
                        .send(Action::ForceFocusChangeRev)
                        .await
                        .unwrap(),
                    ct_event!(keycode press Esc) if !self.body_paragraph_state.is_focused() => {
                        if let Some(tx) = self.action_tx.clone() {
                            let _ = tx.send(Action::ChangeIssueScreen(MainScreen::List)).await;
                        }
                        return;
                    }
                    ct_event!(key press CONTROL-'p') => {
                        self.textbox_state.toggle();
                        match self.textbox_state {
                            InputState::Input => {
                                self.input_state.focus.set(true);
                            }
                            InputState::Preview => {
                                self.paragraph_state.focus.set(true);
                            }
                        }
                    }
                    ct_event!(keycode press Enter) if self.list_state.is_focused() => {
                        self.action_tx
                            .as_ref()
                            .unwrap()
                            .send(Action::ForceFocusChange)
                            .await
                            .unwrap();
                    }
                    ct_event!(keycode press CONTROL-Enter) | ct_event!(keycode press ALT-Enter) => {
                        info!("Enter pressed");
                        let Some(seed) = &self.current else {
                            return;
                        };
                        let body = self.input_state.text();
                        let trimmed = body.trim();
                        if trimmed.is_empty() {
                            self.post_error = Some("Comment cannot be empty.".to_string());
                            return;
                        }
                        self.input_state.set_text("");
                        self.send_comment(seed.number, trimmed.to_string()).await;
                        return;
                    }
                    event::Event::Key(key) if key.code != event::KeyCode::Tab => {
                        let o = self.input_state.handle(event, rat_widget::event::Regular);
                        if o == TextOutcome::TextChanged {
                            self.action_tx
                                .as_ref()
                                .unwrap()
                                .send(Action::ForceRender)
                                .await
                                .unwrap();
                        }
                    }
                    _ => {}
                }
                self.body_paragraph_state
                    .handle(event, rat_widget::event::Regular);
                let outcome = self.list_state.handle(event, rat_widget::event::Regular);
                if outcome == rat_widget::event::Outcome::Changed {
                    self.body_paragraph_state.set_line_offset(0);
                }
            }
            Action::EnterIssueDetails { seed } => {
                let number = seed.number;
                self.current = Some(seed);
                self.post_error = None;
                self.body_cache = None;
                self.body_cache_number = Some(number);
                self.body_paragraph_state.set_line_offset(0);
                if self.cache_number != Some(number) {
                    self.cache_number = None;
                    self.cache_comments.clear();
                    self.markdown_cache.clear();
                }
                if self.cache_number == Some(number) {
                    self.loading.remove(&number);
                    self.error = None;
                } else {
                    self.fetch_comments(number).await;
                }
            }
            Action::IssueCommentsLoaded { number, comments } => {
                self.loading.remove(&number);
                if self.current.as_ref().is_some_and(|s| s.number == number) {
                    self.cache_number = Some(number);
                    info!("Setting {} comments for #{}", comments.len(), number);
                    self.cache_comments = comments;
                    self.markdown_cache.clear();
                    self.body_cache = None;
                    self.body_paragraph_state.set_line_offset(0);
                    self.error = None;
                    self.action_tx
                        .as_ref()
                        .unwrap()
                        .send(Action::ForceRender)
                        .await
                        .unwrap();
                }
            }
            Action::IssueCommentPosted { number, comment } => {
                self.posting = false;
                if self.current.as_ref().is_some_and(|s| s.number == number) {
                    if self.cache_number == Some(number) {
                        self.cache_comments.push(comment);
                    } else {
                        self.cache_number = Some(number);
                        self.cache_comments.clear();
                        self.cache_comments.push(comment);
                        self.markdown_cache.clear();
                        self.body_cache = None;
                    }
                }
            }
            Action::IssueCommentsError { number, message } => {
                self.loading.remove(&number);
                if self.current.as_ref().is_some_and(|s| s.number == number) {
                    self.error = Some(message);
                }
            }
            Action::IssueCommentPostError { number, message } => {
                self.posting = false;
                if self.current.as_ref().is_some_and(|s| s.number == number) {
                    self.post_error = Some(message);
                }
            }
            Action::ChangeIssueScreen(screen) => {
                self.screen = screen;
                match screen {
                    MainScreen::List => {
                        self.input_state.focus.set(false);
                        self.list_state.focus.set(false);
                    }
                    MainScreen::Details => {}
                }
            }
            Action::Tick => {
                if self.is_loading_current() {
                    self.throbber_state.calc_next();
                }
                if self.posting {
                    self.post_throbber_state.calc_next();
                }
            }
            _ => {}
        }
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.input_state.screen_cursor()
    }

    fn should_render(&self) -> bool {
        self.screen == MainScreen::Details
    }

    fn is_animating(&self) -> bool {
        self.screen == MainScreen::Details && (self.is_loading_current() || self.posting)
    }

    fn capture_focus_event(&self, event: &crossterm::event::Event) -> bool {
        if self.screen != MainScreen::Details {
            return false;
        }
        if !self.input_state.is_focused() {
            return false;
        }
        match event {
            crossterm::event::Event::Key(key) => matches!(
                key.code,
                crossterm::event::KeyCode::Tab
                    | crossterm::event::KeyCode::BackTab
                    | crossterm::event::KeyCode::Char('q')
            ),
            _ => false,
        }
    }
    fn set_index(&mut self, index: usize) {
        self.index = index;
    }
}

impl HasFocus for IssueConversation {
    fn build(&self, builder: &mut FocusBuilder) {
        let tag = builder.start(self);
        builder.widget(&self.list_state);
        builder.widget(&self.body_paragraph_state);
        match self.textbox_state {
            InputState::Input => builder.widget(&self.input_state),
            InputState::Preview => builder.widget(&self.paragraph_state),
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
        if self.screen == MainScreen::Details {
            Navigation::Regular
        } else {
            Navigation::None
        }
    }
}

fn build_comment_item(
    author: &str,
    created_at: &str,
    preview: &str,
    is_self: bool,
) -> ListItem<'static> {
    let author_style = if is_self {
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Cyan)
    };
    let header = Line::from(vec![
        Span::styled(author.to_string(), author_style),
        Span::raw("  "),
        Span::styled(created_at.to_string(), Style::new().dim()),
    ]);
    let preview_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(preview.to_string(), Style::new().dim()),
    ]);
    ListItem::new(vec![header, preview_line])
}

fn build_comment_preview_item(
    author: &str,
    created_at: &str,
    body_lines: &[Line<'static>],
    preview_width: usize,
    is_self: bool,
) -> ListItem<'static> {
    let preview = extract_preview(body_lines, preview_width);
    build_comment_item(author, created_at, &preview, is_self)
}

fn extract_preview(lines: &[Line<'static>], preview_width: usize) -> String {
    for line in lines {
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return truncate_preview(trimmed, preview_width.max(8));
        }
    }
    "(empty)".to_string()
}

fn truncate_preview(input: &str, max_width: usize) -> String {
    if display_width(input) <= max_width {
        return input.to_string();
    }
    let mut out = String::new();
    for ch in input.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        if display_width(&candidate) + 3 > max_width {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

fn render_markdown_lines(text: &str, width: usize, indent: usize) -> Vec<Line<'static>> {
    let mut renderer = MarkdownRenderer::new(width, indent);
    let parser = Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH);
    for event in parser {
        match event {
            Event::Start(tag) => renderer.start_tag(tag),
            Event::End(tag) => renderer.end_tag(tag),
            Event::Text(text) => renderer.text(&text),
            Event::Code(text) => renderer.inline_code(&text),
            Event::SoftBreak => renderer.soft_break(),
            Event::HardBreak => renderer.hard_break(),
            Event::Html(text) => renderer.text(&text),
            _ => {}
        }
    }
    renderer.finish()
}

struct MarkdownRenderer {
    lines: Vec<Line<'static>>,
    current_line: Vec<Span<'static>>,
    current_width: usize,
    max_width: usize,
    indent: usize,
    style_stack: Vec<Style>,
    current_style: Style,
    in_block_quote: bool,
    in_code_block: bool,
    list_prefix: Option<String>,
    pending_space: bool,
}

impl MarkdownRenderer {
    fn new(max_width: usize, indent: usize) -> Self {
        Self {
            lines: Vec::new(),
            current_line: Vec::new(),
            current_width: 0,
            max_width: max_width.max(10),
            indent,
            style_stack: Vec::new(),
            current_style: Style::new(),
            in_block_quote: false,
            in_code_block: false,
            list_prefix: None,
            pending_space: false,
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Emphasis => self.push_style(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Link { .. } => self.push_style(
                Style::new()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::UNDERLINED),
            ),
            Tag::Heading { .. } => {
                self.push_style(Style::new().add_modifier(Modifier::BOLD));
            }
            Tag::BlockQuote(_) => {
                self.flush_line();
                self.in_block_quote = true;
            }
            Tag::CodeBlock(..) => {
                self.flush_line();
                self.in_code_block = true;
            }
            Tag::Item => {
                self.flush_line();
                self.list_prefix = Some("• ".to_string());
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Link | TagEnd::Heading(_) => {
                self.pop_style();
            }
            TagEnd::BlockQuote => {
                self.flush_line();
                self.in_block_quote = false;
                self.push_blank_line();
            }
            TagEnd::CodeBlock => {
                self.flush_line();
                self.in_code_block = false;
                self.push_blank_line();
            }
            TagEnd::Item => {
                self.flush_line();
                self.list_prefix = None;
            }
            TagEnd::Paragraph => {
                self.flush_line();
                self.push_blank_line();
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if self.in_code_block {
            self.code_block_text(text);
        } else {
            let style = self.current_style;
            self.push_text(text, style);
        }
    }

    fn inline_code(&mut self, text: &str) {
        let style = self
            .current_style
            .patch(Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        self.push_text(text, style);
    }

    fn soft_break(&mut self) {
        if self.in_code_block {
            self.hard_break();
        } else {
            self.pending_space = true;
        }
    }

    fn hard_break(&mut self) {
        self.flush_line();
    }

    fn push_text(&mut self, text: &str, style: Style) {
        let mut buffer = String::new();
        for ch in text.chars() {
            if ch == '\n' {
                if !buffer.is_empty() {
                    self.push_word(&buffer, style);
                    buffer.clear();
                }
                self.flush_line();
                continue;
            }
            if ch.is_whitespace() {
                if !buffer.is_empty() {
                    self.push_word(&buffer, style);
                    buffer.clear();
                }
                self.pending_space = true;
            } else {
                buffer.push(ch);
            }
        }
        if !buffer.is_empty() {
            self.push_word(&buffer, style);
        }
    }

    fn push_word(&mut self, word: &str, style: Style) {
        let prefix_width = self.prefix_width();
        let max_width = self.max_width;
        let word_width = display_width(word);
        let space_width = if self.pending_space && self.current_width > prefix_width {
            1
        } else {
            0
        };

        if word_width > max_width.saturating_sub(prefix_width) {
            self.push_long_word(word, style);
            self.pending_space = false;
            return;
        }

        if self.current_line.is_empty() {
            self.start_line();
        }

        if self.current_width + space_width + word_width > max_width
            && self.current_width > prefix_width
        {
            self.flush_line();
            self.start_line();
        }

        if self.pending_space && self.current_width > prefix_width {
            self.current_line.push(Span::raw(" "));
            self.current_width += 1;
        }
        self.pending_space = false;

        self.current_line
            .push(Span::styled(word.to_string(), style));
        self.current_width += word_width;
    }

    fn push_long_word(&mut self, word: &str, style: Style) {
        let available = self.max_width.saturating_sub(self.prefix_width()).max(1);
        let wrapped = textwrap::wrap(word, textwrap::Options::new(available).break_words(true));
        for (idx, part) in wrapped.iter().enumerate() {
            if idx > 0 {
                self.flush_line();
            }
            if self.current_line.is_empty() {
                self.start_line();
            }
            self.current_line
                .push(Span::styled(part.to_string(), style));
            self.current_width += display_width(part);
        }
    }

    fn code_block_text(&mut self, text: &str) {
        let style = Style::new().fg(Color::LightYellow);
        for line in text.split('\n') {
            self.flush_line();
            self.start_line();
            self.current_line
                .push(Span::styled(line.to_string(), style));
            self.current_width += display_width(line);
            self.flush_line();
        }
    }

    fn start_line(&mut self) {
        if !self.current_line.is_empty() {
            return;
        }
        if self.indent > 0 {
            let indent = " ".repeat(self.indent);
            self.current_width += self.indent;
            self.current_line.push(Span::raw(indent));
        }
        if self.in_block_quote {
            self.current_width += 2;
            self.current_line
                .push(Span::styled("│ ", Style::new().fg(Color::DarkGray)));
        }
        if let Some(prefix) = &self.list_prefix {
            self.current_width += display_width(prefix);
            self.current_line.push(Span::raw(prefix.clone()));
        }
    }

    fn prefix_width(&self) -> usize {
        let mut width = self.indent;
        if self.in_block_quote {
            width += 2;
        }
        if let Some(prefix) = &self.list_prefix {
            width += display_width(prefix);
        }
        width
    }

    fn flush_line(&mut self) {
        if self.current_line.is_empty() {
            self.pending_space = false;
            return;
        }
        let line = Line::from(std::mem::take(&mut self.current_line));
        self.lines.push(line);
        self.current_width = 0;
        self.pending_space = false;
    }

    fn push_blank_line(&mut self) {
        if self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            return;
        }
        self.lines.push(Line::from(Vec::<Span<'static>>::new()));
    }

    fn push_style(&mut self, style: Style) {
        self.style_stack.push(self.current_style);
        self.current_style = self.current_style.patch(style);
    }

    fn pop_style(&mut self) {
        if let Some(prev) = self.style_stack.pop() {
            self.current_style = prev;
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_line();
        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }
        if self.lines.is_empty() {
            self.lines.push(Line::from(vec![Span::raw("")]));
        }
        self.lines
    }
}
