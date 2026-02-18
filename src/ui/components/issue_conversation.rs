use async_trait::async_trait;
use crossterm::event;
use futures::{StreamExt, stream};
use octocrab::models::{
    CommentId, IssueState, issues::Comment as ApiComment, reactions::ReactionContent,
};
use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, Event, Options, Parser, Tag, TagEnd, TextMergeStream,
};
use rat_cursor::HasScreenCursor;
use rat_widget::{
    event::{HandleEvent, Outcome, TextOutcome, ct_event},
    focus::{FocusBuilder, FocusFlag, HasFocus, Navigation},
    list::{ListState, selection::RowSelection},
    paragraph::{Paragraph, ParagraphState},
    textarea::{TextArea, TextAreaState, TextWrap},
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{self, Block, ListItem, StatefulWidget, Widget},
};
use ratatui_macros::{horizontal, line, vertical};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};
use textwrap::{core::display_width, wrap};
use throbber_widgets_tui::{BRAILLE_SIX_DOUBLE, Throbber, ThrobberState, WhichUse};
use tracing::trace;

use crate::{
    app::GITHUB_CLIENT,
    errors::AppError,
    ui::{
        Action,
        components::{
            Component,
            help::HelpElementKind,
            issue_list::{IssueClosePopupState, MainScreen, render_issue_close_popup},
        },
        layout::Layout,
        utils::get_border_style,
        widgets::link::Link,
    },
};
use anyhow::anyhow;

pub const HELP: &[HelpElementKind] = &[
    crate::help_text!("Issue Conversation Help"),
    crate::help_keybind!("Up/Down", "select issue body/comment entry"),
    crate::help_keybind!("PageUp/PageDown/Home/End", "scroll message body pane"),
    crate::help_keybind!("f", "toggle fullscreen body view"),
    crate::help_keybind!("C", "close selected issue"),
    crate::help_keybind!("Enter (popup)", "confirm close reason"),
    crate::help_keybind!("Ctrl+P", "toggle comment input/preview"),
    crate::help_keybind!("e", "edit selected comment in external editor"),
    crate::help_keybind!("r", "add reaction to selected comment"),
    crate::help_keybind!("R", "remove reaction from selected comment"),
    crate::help_keybind!("Ctrl+Enter / Alt+Enter", "send comment"),
    crate::help_keybind!("Esc", "exit fullscreen / return to issue list"),
];

struct SyntectAssets {
    syntaxes: SyntaxSet,
    theme: Theme,
}

static SYNTECT_ASSETS: OnceLock<SyntectAssets> = OnceLock::new();

fn syntect_assets() -> &'static SyntectAssets {
    SYNTECT_ASSETS.get_or_init(|| {
        let syntaxes = SyntaxSet::load_defaults_nonewlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set
            .themes
            .get("base16-ocean.dark")
            .or_else(|| theme_set.themes.values().next())
            .cloned()
            .expect("syntect default theme set should include at least one theme");
        SyntectAssets { syntaxes, theme }
    })
}

#[derive(Debug, Clone)]
pub struct IssueConversationSeed {
    pub number: u64,
    pub author: Arc<str>,
    pub created_at: Arc<str>,
    pub body: Option<Arc<str>>,
    pub title: Option<Arc<str>>,
}

impl IssueConversationSeed {
    pub fn from_issue(issue: &octocrab::models::issues::Issue) -> Self {
        Self {
            number: issue.number,
            author: Arc::<str>::from(issue.user.login.as_str()),
            created_at: Arc::<str>::from(issue.created_at.format("%Y-%m-%d %H:%M").to_string()),
            body: issue.body.as_ref().map(|b| Arc::<str>::from(b.as_str())),
            title: Some(Arc::<str>::from(issue.title.as_str())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommentView {
    pub id: u64,
    pub author: Arc<str>,
    pub created_at: Arc<str>,
    pub body: Arc<str>,
    pub reactions: Option<Vec<(ReactionContent, u64)>>,
    pub my_reactions: Option<Vec<ReactionContent>>,
}

impl CommentView {
    pub fn from_api(comment: ApiComment) -> Self {
        let body = comment.body.unwrap_or_default();
        Self {
            id: comment.id.0,
            author: Arc::<str>::from(comment.user.login.as_str()),
            created_at: Arc::<str>::from(comment.created_at.format("%Y-%m-%d %H:%M").to_string()),
            body: Arc::<str>::from(body),
            reactions: None,
            my_reactions: None,
        }
    }
}

pub struct IssueConversation {
    title: Option<Arc<str>>,
    action_tx: Option<tokio::sync::mpsc::Sender<Action>>,
    current: Option<IssueConversationSeed>,
    cache_number: Option<u64>,
    cache_comments: Vec<CommentView>,
    markdown_cache: HashMap<u64, MarkdownRender>,
    body_cache: Option<MarkdownRender>,
    body_cache_number: Option<u64>,
    markdown_width: usize,
    loading: HashSet<u64>,
    posting: bool,
    error: Option<String>,
    post_error: Option<String>,
    reaction_error: Option<String>,
    close_error: Option<String>,
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
    reaction_mode: Option<ReactionMode>,
    close_popup: Option<IssueClosePopupState>,
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

#[derive(Debug, Clone, Default)]
struct MarkdownRender {
    lines: Vec<Line<'static>>,
    links: Vec<RenderedLink>,
}

#[derive(Debug, Clone)]
struct RenderedLink {
    line: usize,
    col: usize,
    label: String,
    url: String,
    width: usize,
}

#[derive(Debug, Clone)]
enum ReactionMode {
    Add {
        comment_id: u64,
        selected: usize,
    },
    Remove {
        comment_id: u64,
        selected: usize,
        options: Vec<ReactionContent>,
    },
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
    fn in_details_mode(&self) -> bool {
        matches!(
            self.screen,
            MainScreen::Details | MainScreen::DetailsFullscreen
        )
    }

    pub fn new(app_state: crate::ui::AppState) -> Self {
        Self {
            title: None,
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
            reaction_error: None,
            close_error: None,
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
            reaction_mode: None,
            close_popup: None,
            index: 0,
        }
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        if self.screen == MainScreen::DetailsFullscreen {
            self.area = area.main_content;
            self.render_body(area.main_content, buf);
            return;
        }
        self.area = area.main_content;
        let title = self.title.clone().unwrap_or_default();
        let wrapped_title = wrap(&title, area.main_content.width.saturating_sub(2) as usize);
        let title_para_height = wrapped_title.len() as u16 + 2;
        let title_para = Text::from_iter(wrapped_title);

        let areas = vertical![==title_para_height, *=1, ==5].split(area.main_content);
        let title_area = areas[0];
        let content_area = areas[1];
        let input_area = areas[2];
        let content_split = horizontal![*=1, *=1].split(content_area);
        let list_area = content_split[0];
        let body_area = content_split[1];
        let items = self.build_items(list_area, body_area);

        let title_widget = widgets::Paragraph::new(title_para)
            .block(Block::bordered().border_type(ratatui::widgets::BorderType::Rounded))
            .style(Style::default().add_modifier(Modifier::BOLD));
        title_widget.render(title_area, buf);

        let mut list_block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&self.list_state));

        if !self.is_loading_current() {
            let mut title = format!("[{}] Conversation", self.index);
            if let Some(prompt) = self.reaction_mode_prompt() {
                title.push_str(" | ");
                title.push_str(&prompt);
            } else if let Some(err) = &self.reaction_error {
                title.push_str(" | ");
                title.push_str(err);
            } else if let Some(err) = &self.close_error {
                title.push_str(" | ");
                title.push_str(err);
            }
            list_block = list_block.title(title);
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
        self.render_close_popup(area.main_content, buf);
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
                .get_or_insert_with(|| render_markdown(body, width, 2));
            items.push(build_comment_preview_item(
                seed.author.as_ref(),
                seed.created_at.as_ref(),
                &body_lines.lines,
                preview_width,
                seed.author.as_ref() == self.current_user,
                None,
            ));
            self.message_keys.push(MessageKey::IssueBody(seed.number));
        }

        if self.cache_number == Some(seed.number) {
            trace!(
                "Rendering {} comments for #{}",
                self.cache_comments.len(),
                seed.number
            );
            for comment in &self.cache_comments {
                let body_lines = self
                    .markdown_cache
                    .entry(comment.id)
                    .or_insert_with(|| render_markdown(comment.body.as_ref(), width, 2));
                items.push(build_comment_preview_item(
                    comment.author.as_ref(),
                    comment.created_at.as_ref(),
                    &body_lines.lines,
                    preview_width,
                    comment.author.as_ref() == self.current_user,
                    comment.reactions.as_deref(),
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
        let selected_body = self.selected_body_render().cloned();
        let body_lines: Vec<Line<'static>> = selected_body
            .as_ref()
            .map(|v| v.lines.clone())
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
                    .title(if self.screen == MainScreen::DetailsFullscreen {
                        "Message Body (PageUp/PageDown/Home/End | f/Esc: exit fullscreen)"
                    } else {
                        "Message Body (PageUp/PageDown/Home/End)"
                    }),
            )
            .focus_style(Style::default())
            .hide_focus(true);

        body.render(body_area, buf, &mut self.body_paragraph_state);

        if let Some(render) = selected_body.as_ref() {
            self.render_body_links(body_area, buf, render);
        }
    }

    fn selected_body_render(&self) -> Option<&MarkdownRender> {
        let selected = self.list_state.selected_checked()?;
        let key = self.message_keys.get(selected)?;
        match key {
            MessageKey::IssueBody(number) => {
                if self.body_cache_number == Some(*number) {
                    self.body_cache.as_ref()
                } else {
                    None
                }
            }
            MessageKey::Comment(id) => self.markdown_cache.get(id),
        }
    }

    fn render_body_links(&self, body_area: Rect, buf: &mut Buffer, render: &MarkdownRender) {
        if render.links.is_empty() {
            return;
        }

        let inner = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .inner(body_area);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let line_offset = self.body_paragraph_state.line_offset();
        for link in &render.links {
            if link.line < line_offset {
                continue;
            }

            let local_y = link.line - line_offset;
            if local_y >= inner.height as usize || link.col >= inner.width as usize {
                continue;
            }

            let available = (inner.width as usize).saturating_sub(link.col);
            if available == 0 {
                continue;
            }

            let link_area = Rect {
                x: inner.x + link.col as u16,
                y: inner.y + local_y as u16,
                width: (available.min(link.width)) as u16,
                height: 1,
            };
            Link::new(link.label.as_str(), link.url.as_str())
                .style(
                    Style::new()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::UNDERLINED),
                )
                .render(link_area, buf);
        }
    }

    fn selected_comment_id(&self) -> Option<u64> {
        let selected = self.list_state.selected_checked()?;
        match self.message_keys.get(selected)? {
            MessageKey::Comment(id) => Some(*id),
            MessageKey::IssueBody(_) => None,
        }
    }

    fn selected_comment(&self) -> Option<&CommentView> {
        let id = self.selected_comment_id()?;
        self.cache_comments.iter().find(|c| c.id == id)
    }

    async fn open_external_editor_for_comment(
        &mut self,
        issue_number: u64,
        comment_id: u64,
        initial_body: String,
    ) {
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        if action_tx
            .send(Action::EditorModeChanged(true))
            .await
            .is_err()
        {
            return;
        }

        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                ratatui::restore();
                let edited = edit::edit(&initial_body).map_err(|err| err.to_string());
                let _ = ratatui::init();
                edited
            })
            .await
            .map_err(|err| err.to_string())
            .and_then(|edited| edited.map_err(|err| err.replace('\n', " ")));

            let _ = action_tx.send(Action::EditorModeChanged(false)).await;
            let _ = action_tx
                .send(Action::IssueCommentEditFinished {
                    issue_number,
                    comment_id,
                    result,
                })
                .await;
            let _ = action_tx.send(Action::ForceRender).await;
        });
    }

    async fn patch_comment(&mut self, issue_number: u64, comment_id: u64, body: String) {
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();

        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::IssueCommentEditFinished {
                        issue_number,
                        comment_id,
                        result: Err("GitHub client not initialized.".to_string()),
                    })
                    .await;
                return;
            };

            let handler = client.inner().issues(owner, repo);
            match handler.update_comment(CommentId(comment_id), body).await {
                Ok(comment) => {
                    let _ = action_tx
                        .send(Action::IssueCommentPatched {
                            issue_number,
                            comment: CommentView::from_api(comment),
                        })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::IssueCommentEditFinished {
                            issue_number,
                            comment_id,
                            result: Err(err.to_string().replace('\n', " ")),
                        })
                        .await;
                }
            }
        });
    }

    fn reaction_mode_prompt(&self) -> Option<String> {
        let mode = self.reaction_mode.as_ref()?;
        match mode {
            ReactionMode::Add { selected, .. } => Some(format!(
                "Add reaction: {}",
                format_reaction_picker(*selected, &reaction_add_options())
            )),
            ReactionMode::Remove {
                selected, options, ..
            } => Some(format!(
                "Remove reaction: {}",
                format_reaction_picker(*selected, options)
            )),
        }
    }

    fn open_close_popup(&mut self) {
        let Some(seed) = &self.current else {
            self.close_error = Some("No issue selected.".to_string());
            return;
        };
        self.close_error = None;
        self.close_popup = Some(IssueClosePopupState::new(seed.number));
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

    async fn handle_close_popup_event(&mut self, event: &event::Event) -> bool {
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

    fn start_add_reaction_mode(&mut self) {
        let Some(comment_id) = self.selected_comment_id() else {
            self.reaction_error = Some("Select a comment to add a reaction.".to_string());
            return;
        };
        self.reaction_error = None;
        self.reaction_mode = Some(ReactionMode::Add {
            comment_id,
            selected: 0,
        });
    }

    fn start_remove_reaction_mode(&mut self) {
        let Some(comment) = self.selected_comment() else {
            self.reaction_error = Some("Select a comment to remove a reaction.".to_string());
            return;
        };
        let comment_id = comment.id;
        let mut options = comment.my_reactions.as_ref().cloned().unwrap_or_default();

        options.sort_by_key(reaction_order);
        options.dedup();
        if options.is_empty() {
            self.reaction_error = Some("No reactions available to remove.".to_string());
            return;
        }
        self.reaction_error = None;
        self.reaction_mode = Some(ReactionMode::Remove {
            comment_id,
            selected: 0,
            options,
        });
    }

    async fn handle_reaction_mode_event(&mut self, event: &event::Event) -> bool {
        let Some(mode) = &mut self.reaction_mode else {
            return false;
        };

        let mut submit: Option<(u64, ReactionContent, bool)> = None;
        match event {
            ct_event!(keycode press Esc) => {
                self.reaction_mode = None;
                return true;
            }
            ct_event!(keycode press Up) => match mode {
                ReactionMode::Add { selected, .. } => {
                    let len = reaction_add_options().len();
                    if len > 0 {
                        *selected = if *selected == 0 {
                            len - 1
                        } else {
                            *selected - 1
                        };
                    }
                    return true;
                }
                ReactionMode::Remove {
                    selected, options, ..
                } => {
                    let len = options.len();
                    if len > 0 {
                        *selected = if *selected == 0 {
                            len - 1
                        } else {
                            *selected - 1
                        };
                    }
                    return true;
                }
            },
            ct_event!(keycode press Down) => match mode {
                ReactionMode::Add { selected, .. } => {
                    let len = reaction_add_options().len();
                    if len > 0 {
                        *selected = (*selected + 1) % len;
                    }
                    return true;
                }
                ReactionMode::Remove {
                    selected, options, ..
                } => {
                    let len = options.len();
                    if len > 0 {
                        *selected = (*selected + 1) % len;
                    }
                    return true;
                }
            },
            ct_event!(keycode press Enter) => match mode {
                ReactionMode::Add {
                    comment_id,
                    selected,
                } => {
                    let options = reaction_add_options();
                    if let Some(content) = options.get(*selected).cloned() {
                        submit = Some((*comment_id, content, true));
                    }
                }
                ReactionMode::Remove {
                    comment_id,
                    selected,
                    options,
                } => {
                    if let Some(content) = options.get(*selected).cloned() {
                        submit = Some((*comment_id, content, false));
                    }
                }
            },
            _ => return false,
        }

        if let Some((comment_id, content, add)) = submit {
            self.reaction_mode = None;
            self.reaction_error = None;
            if add {
                self.add_reaction(comment_id, content).await;
            } else {
                self.remove_reaction(comment_id, content).await;
            }
            return true;
        }
        true
    }

    fn is_loading_current(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|seed| self.loading.contains(&seed.number))
    }

    async fn add_reaction(&mut self, comment_id: u64, content: ReactionContent) {
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let current_user = self.current_user.clone();
        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::IssueReactionEditError {
                        comment_id,
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let handler = client.inner().issues(owner, repo);
            if let Err(err) = handler.create_comment_reaction(comment_id, content).await {
                let _ = action_tx
                    .send(Action::IssueReactionEditError {
                        comment_id,
                        message: err.to_string().replace('\n', " "),
                    })
                    .await;
                return;
            }

            match handler.list_comment_reactions(comment_id).send().await {
                Ok(mut page) => {
                    let (counts, mine) =
                        to_reaction_snapshot(std::mem::take(&mut page.items), &current_user);
                    let mut reactions = HashMap::new();
                    let mut own_reactions = HashMap::new();
                    reactions.insert(comment_id, counts);
                    own_reactions.insert(comment_id, mine);
                    let _ = action_tx
                        .send(Action::IssueReactionsLoaded {
                            reactions,
                            own_reactions,
                        })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::IssueReactionEditError {
                            comment_id,
                            message: err.to_string().replace('\n', " "),
                        })
                        .await;
                }
            }
        });
    }

    async fn remove_reaction(&mut self, comment_id: u64, content: ReactionContent) {
        let Some(action_tx) = self.action_tx.clone() else {
            return;
        };
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let current_user = self.current_user.clone();
        tokio::spawn(async move {
            let Some(client) = GITHUB_CLIENT.get() else {
                let _ = action_tx
                    .send(Action::IssueReactionEditError {
                        comment_id,
                        message: "GitHub client not initialized.".to_string(),
                    })
                    .await;
                return;
            };
            let handler = client.inner().issues(owner, repo);
            match handler.list_comment_reactions(comment_id).send().await {
                Ok(mut page) => {
                    let mut items = std::mem::take(&mut page.items);
                    let to_delete = items
                        .iter()
                        .find(|reaction| {
                            reaction.content == content
                                && reaction.user.login.eq_ignore_ascii_case(&current_user)
                        })
                        .map(|reaction| reaction.id);

                    let Some(reaction_id) = to_delete else {
                        let _ = action_tx
                            .send(Action::IssueReactionEditError {
                                comment_id,
                                message: "No matching reaction from current user.".to_string(),
                            })
                            .await;
                        return;
                    };

                    if let Err(err) = handler
                        .delete_comment_reaction(comment_id, reaction_id)
                        .await
                    {
                        let _ = action_tx
                            .send(Action::IssueReactionEditError {
                                comment_id,
                                message: err.to_string().replace('\n', " "),
                            })
                            .await;
                        return;
                    }

                    let mut removed = false;
                    let (counts, mine) = to_reaction_snapshot(
                        items.drain(..).filter_map(|reaction| {
                            if !removed
                                && reaction.content == content
                                && reaction.user.login.eq_ignore_ascii_case(&current_user)
                            {
                                removed = true;
                                None
                            } else {
                                Some(reaction)
                            }
                        }),
                        &current_user,
                    );
                    let mut reactions = HashMap::new();
                    let mut own_reactions = HashMap::new();
                    reactions.insert(comment_id, counts);
                    own_reactions.insert(comment_id, mine);
                    let _ = action_tx
                        .send(Action::IssueReactionsLoaded {
                            reactions,
                            own_reactions,
                        })
                        .await;
                }
                Err(err) => {
                    let _ = action_tx
                        .send(Action::IssueReactionEditError {
                            comment_id,
                            message: err.to_string().replace('\n', " "),
                        })
                        .await;
                }
            }
        });
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
        let current_user = self.current_user.clone();
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
                    let comments = std::mem::take(&mut p.items);
                    let comment_ids = comments.iter().map(|c| c.id.0).collect::<Vec<_>>();
                    let comments: Vec<CommentView> =
                        comments.into_iter().map(CommentView::from_api).collect();
                    trace!("Loaded {} comments for issue {}", comments.len(), number);
                    let _ = action_tx
                        .send(Action::IssueCommentsLoaded { number, comments })
                        .await;
                    let refer = &handler;
                    let current_user = current_user.clone();
                    let reaction_snapshots = stream::iter(comment_ids)
                        .filter_map(|id| {
                            let current_user = current_user.clone();
                            async move {
                                let reactions = refer.list_comment_reactions(id).send().await;
                                let mut page = reactions.ok()?;
                                Some((
                                    id,
                                    to_reaction_snapshot(
                                        std::mem::take(&mut page.items),
                                        &current_user,
                                    ),
                                ))
                            }
                        })
                        .collect::<HashMap<_, _>>()
                        .await;
                    let mut reactions = HashMap::with_capacity(reaction_snapshots.len());
                    let mut own_reactions = HashMap::with_capacity(reaction_snapshots.len());
                    for (id, (counts, mine)) in reaction_snapshots {
                        reactions.insert(id, counts);
                        own_reactions.insert(id, mine);
                    }
                    let _ = action_tx
                        .send(Action::IssueReactionsLoaded {
                            reactions,
                            own_reactions,
                        })
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

    async fn handle_event(&mut self, event: Action) -> Result<(), AppError> {
        match event {
            Action::AppEvent(ref event) => {
                if !self.in_details_mode() {
                    return Ok(());
                }
                if self.screen == MainScreen::DetailsFullscreen
                    && matches!(
                        event,
                        ct_event!(key press 'f') | ct_event!(keycode press Esc)
                    )
                {
                    if let Some(tx) = self.action_tx.clone() {
                        let _ = tx
                            .send(Action::ChangeIssueScreen(MainScreen::Details))
                            .await;
                    }
                    return Ok(());
                }
                if self.handle_close_popup_event(event).await {
                    return Ok(());
                }
                if self.handle_reaction_mode_event(event).await {
                    return Ok(());
                }

                match event {
                    event::Event::Key(key)
                        if key.code == event::KeyCode::Char('f')
                            && key.modifiers == event::KeyModifiers::NONE
                            && self.screen == MainScreen::Details
                            && self.body_paragraph_state.is_focused() =>
                    {
                        if let Some(tx) = self.action_tx.clone() {
                            let _ = tx
                                .send(Action::ChangeIssueScreen(MainScreen::DetailsFullscreen))
                                .await;
                        }
                        return Ok(());
                    }
                    event::Event::Key(key)
                        if key.code == event::KeyCode::Char('e')
                            && key.modifiers == event::KeyModifiers::NONE
                            && (self.list_state.is_focused()
                                || self.body_paragraph_state.is_focused()) =>
                    {
                        let seed = self.current.as_ref().ok_or_else(|| {
                            AppError::Other(anyhow!("no issue selected for comment editing"))
                        })?;
                        let comment = self
                            .selected_comment()
                            .ok_or_else(|| AppError::Other(anyhow!("select a comment to edit")))?;
                        self.open_external_editor_for_comment(
                            seed.number,
                            comment.id,
                            comment.body.to_string(),
                        )
                        .await;
                        return Ok(());
                    }
                    event::Event::Key(key)
                        if key.code == event::KeyCode::Char('r')
                            && key.modifiers == event::KeyModifiers::NONE
                            && self.list_state.is_focused() =>
                    {
                        self.start_add_reaction_mode();
                        return Ok(());
                    }
                    event::Event::Key(key)
                        if key.code == event::KeyCode::Char('R')
                            && self.list_state.is_focused() =>
                    {
                        self.start_remove_reaction_mode();
                        return Ok(());
                    }
                    event::Event::Key(key)
                        if key.code == event::KeyCode::Char('C')
                            && (self.list_state.is_focused()
                                || self.body_paragraph_state.is_focused()) =>
                    {
                        self.open_close_popup();
                        return Ok(());
                    }
                    ct_event!(keycode press Tab) | ct_event!(keycode press BackTab)
                        if self.input_state.is_focused() =>
                    {
                        let action_tx = self.action_tx.as_ref().ok_or_else(|| {
                            AppError::Other(anyhow!(
                                "issue conversation action channel unavailable"
                            ))
                        })?;
                        action_tx
                            .send(Action::ForceFocusChange)
                            .await
                            .map_err(|_| AppError::TokioMpsc)?;
                    }
                    ct_event!(keycode press Esc) if self.body_paragraph_state.is_focused() => {
                        let action_tx = self.action_tx.as_ref().ok_or_else(|| {
                            AppError::Other(anyhow!(
                                "issue conversation action channel unavailable"
                            ))
                        })?;
                        action_tx
                            .send(Action::ForceFocusChangeRev)
                            .await
                            .map_err(|_| AppError::TokioMpsc)?;
                    }
                    ct_event!(keycode press Esc) if !self.body_paragraph_state.is_focused() => {
                        if let Some(tx) = self.action_tx.clone() {
                            let _ = tx.send(Action::ChangeIssueScreen(MainScreen::List)).await;
                        }
                        return Ok(());
                    }
                    ct_event!(key press CONTROL-'p') => {
                        self.textbox_state.toggle();
                        match self.textbox_state {
                            InputState::Input => {
                                self.input_state.focus.set(true);
                                self.paragraph_state.focus.set(false);
                            }
                            InputState::Preview => {
                                self.input_state.focus.set(false);
                                self.paragraph_state.focus.set(true);
                            }
                        }
                        if let Some(ref tx) = self.action_tx {
                            let _ = tx.send(Action::ForceRender).await;
                        }
                    }
                    ct_event!(keycode press Enter) if self.list_state.is_focused() => {
                        let action_tx = self.action_tx.as_ref().ok_or_else(|| {
                            AppError::Other(anyhow!(
                                "issue conversation action channel unavailable"
                            ))
                        })?;
                        action_tx
                            .send(Action::ForceFocusChange)
                            .await
                            .map_err(|_| AppError::TokioMpsc)?;
                    }
                    ct_event!(keycode press CONTROL-Enter) | ct_event!(keycode press ALT-Enter) => {
                        trace!("Enter pressed");
                        let Some(seed) = &self.current else {
                            return Ok(());
                        };
                        let body = self.input_state.text();
                        let trimmed = body.trim();
                        if trimmed.is_empty() {
                            self.post_error = Some("Comment cannot be empty.".to_string());
                            return Ok(());
                        }
                        self.input_state.set_text("");
                        self.send_comment(seed.number, trimmed.to_string()).await;
                        return Ok(());
                    }

                    ct_event!(key press '>')
                        if self.list_state.is_focused()
                            || self.body_paragraph_state.is_focused() =>
                    {
                        if let Some(comment) = self.selected_comment() {
                            let comment_body = comment.body.as_ref();
                            let quoted = comment_body
                                .lines()
                                .map(|line| format!("> {}", line.trim()))
                                .collect::<Vec<_>>()
                                .join("\n");
                            self.input_state.insert_str(&quoted);
                            self.input_state.insert_newline();
                            self.input_state.move_to_end(false);
                            self.input_state.move_to_line_end(false);
                            self.input_state.focus.set(true);
                            self.list_state.focus.set(false);
                        }
                    }

                    event::Event::Key(key) if key.code != event::KeyCode::Tab => {
                        let o = self.input_state.handle(event, rat_widget::event::Regular);
                        let o2 = self
                            .paragraph_state
                            .handle(event, rat_widget::event::Regular);
                        if matches!(
                            event,
                            ct_event!(keycode press Up)
                                | ct_event!(keycode press Down)
                                | ct_event!(keycode press Left)
                                | ct_event!(keycode press Right)
                        ) {
                            let action_tx = self.action_tx.as_ref().ok_or_else(|| {
                                AppError::Other(anyhow!(
                                    "issue conversation action channel unavailable"
                                ))
                            })?;
                            action_tx
                                .send(Action::ForceRender)
                                .await
                                .map_err(|_| AppError::TokioMpsc)?;
                        }
                        if o == TextOutcome::TextChanged || o2 == Outcome::Changed {
                            trace!("Input changed, forcing re-render");
                            let action_tx = self.action_tx.as_ref().ok_or_else(|| {
                                AppError::Other(anyhow!(
                                    "issue conversation action channel unavailable"
                                ))
                            })?;
                            action_tx
                                .send(Action::ForceRender)
                                .await
                                .map_err(|_| AppError::TokioMpsc)?;
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
                self.title = seed.title.clone();
                self.current = Some(seed);
                self.post_error = None;
                self.reaction_error = None;
                self.close_error = None;
                self.reaction_mode = None;
                self.close_popup = None;
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
                    trace!("Setting {} comments for #{}", comments.len(), number);
                    self.cache_comments = comments;
                    self.markdown_cache.clear();
                    self.body_cache = None;
                    self.body_paragraph_state.set_line_offset(0);
                    self.error = None;
                    let action_tx = self.action_tx.as_ref().ok_or_else(|| {
                        AppError::Other(anyhow!("issue conversation action channel unavailable"))
                    })?;
                    action_tx
                        .send(Action::ForceRender)
                        .await
                        .map_err(|_| AppError::TokioMpsc)?;
                }
            }
            Action::IssueReactionsLoaded {
                reactions,
                own_reactions,
            } => {
                self.reaction_error = None;
                for (id, reaction_content) in reactions {
                    if let Some(comment) = self.cache_comments.iter_mut().find(|c| c.id == id) {
                        comment.reactions = Some(reaction_content);
                        comment.my_reactions =
                            Some(own_reactions.get(&id).cloned().unwrap_or_else(Vec::new));
                    }
                }
            }
            Action::IssueReactionEditError {
                comment_id: _,
                message,
            } => {
                self.reaction_error = Some(message);
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
            Action::IssueCommentEditFinished {
                issue_number,
                comment_id,
                result,
            } => {
                if self
                    .current
                    .as_ref()
                    .is_none_or(|seed| seed.number != issue_number)
                {
                    return Ok(());
                }
                match result {
                    Ok(body) => {
                        let Some(existing) =
                            self.cache_comments.iter().find(|c| c.id == comment_id)
                        else {
                            return Err(AppError::Other(anyhow!(
                                "selected comment is no longer available"
                            )));
                        };
                        if body == existing.body.as_ref() {
                            return Ok(());
                        }
                        let trimmed = body.trim();
                        if trimmed.is_empty() {
                            return Err(AppError::Other(anyhow!(
                                "comment cannot be empty after editing"
                            )));
                        }
                        self.patch_comment(issue_number, comment_id, trimmed.to_string())
                            .await;
                        if let Some(action_tx) = self.action_tx.as_ref() {
                            action_tx
                                .send(Action::ForceRender)
                                .await
                                .map_err(|_| AppError::TokioMpsc)?;
                        }
                    }
                    Err(message) => {
                        return Err(AppError::Other(anyhow!("comment edit failed: {message}")));
                    }
                }
            }
            Action::IssueCommentPatched {
                issue_number,
                comment,
            } => {
                if self
                    .current
                    .as_ref()
                    .is_some_and(|seed| seed.number == issue_number)
                    && let Some(existing) =
                        self.cache_comments.iter_mut().find(|c| c.id == comment.id)
                {
                    let reactions = existing.reactions.clone();
                    let my_reactions = existing.my_reactions.clone();
                    *existing = comment;
                    existing.reactions = reactions;
                    existing.my_reactions = my_reactions;
                    self.markdown_cache.remove(&existing.id);
                }
            }
            Action::IssueCloseSuccess { issue } => {
                let issue = *issue;
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
                                seed: crate::ui::components::issue_detail::IssuePreviewSeed::from_issue(
                                    &issue,
                                ),
                            })
                            .await;
                        let _ = action_tx.send(Action::RefreshIssueList).await;
                    }
                }
            }
            Action::IssueCloseError { number, message } => {
                if let Some(popup) = self.close_popup.as_mut()
                    && popup.issue_number == number
                {
                    popup.loading = false;
                    popup.error = Some(message.clone());
                    self.close_error = Some(message);
                }
            }
            Action::ChangeIssueScreen(screen) => {
                self.screen = screen;
                match screen {
                    MainScreen::List => {
                        self.input_state.focus.set(false);
                        self.list_state.focus.set(false);
                        self.reaction_mode = None;
                        self.close_popup = None;
                    }
                    MainScreen::Details => {}
                    MainScreen::DetailsFullscreen => {
                        self.list_state.focus.set(false);
                        self.input_state.focus.set(false);
                        self.paragraph_state.focus.set(false);
                        self.body_paragraph_state.focus.set(true);
                    }
                    MainScreen::CreateIssue => {
                        self.input_state.focus.set(false);
                        self.list_state.focus.set(false);
                        self.reaction_mode = None;
                        self.close_popup = None;
                    }
                }
            }
            Action::Tick => {
                if self.is_loading_current() {
                    self.throbber_state.calc_next();
                }
                if self.posting {
                    self.post_throbber_state.calc_next();
                }
                if let Some(popup) = self.close_popup.as_mut()
                    && popup.loading
                {
                    popup.throbber_state.calc_next();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.input_state.screen_cursor()
    }

    fn should_render(&self) -> bool {
        self.in_details_mode()
    }

    fn is_animating(&self) -> bool {
        self.in_details_mode()
            && (self.is_loading_current()
                || self.posting
                || self.close_popup.as_ref().is_some_and(|popup| popup.loading))
    }

    fn capture_focus_event(&self, event: &crossterm::event::Event) -> bool {
        if !self.in_details_mode() {
            return false;
        }
        if self.screen == MainScreen::DetailsFullscreen {
            return true;
        }
        if self.close_popup.is_some() {
            return true;
        }
        if self.input_state.is_focused() {
            return true;
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

    fn set_global_help(&self) {
        if let Some(action_tx) = &self.action_tx {
            let _ = action_tx.try_send(Action::SetHelp(HELP));
        }
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
        if self.in_details_mode() {
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
    reactions: Option<&[(ReactionContent, u64)]>,
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
    let mut lines = vec![header, preview_line];
    if let Some(reactions) = reactions
        && !reactions.is_empty()
    {
        lines.push(build_reactions_line(reactions));
    }
    ListItem::new(lines)
}

fn build_comment_preview_item(
    author: &str,
    created_at: &str,
    body_lines: &[Line<'static>],
    preview_width: usize,
    is_self: bool,
    reactions: Option<&[(ReactionContent, u64)]>,
) -> ListItem<'static> {
    let preview = extract_preview(body_lines, preview_width);
    build_comment_item(author, created_at, &preview, is_self, reactions)
}

fn build_reactions_line(reactions: &[(ReactionContent, u64)]) -> Line<'static> {
    let mut ordered = reactions.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(content, _)| reaction_order(content));

    let mut spans = vec![Span::raw("  ")];
    for (idx, (content, count)) in ordered.into_iter().enumerate() {
        if idx != 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            reaction_label(content).to_string(),
            Style::new().fg(Color::Yellow),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(count.to_string(), Style::new().dim()));
    }
    Line::from(spans)
}

fn reaction_order(content: &ReactionContent) -> usize {
    match content {
        ReactionContent::PlusOne => 0,
        ReactionContent::Heart => 1,
        ReactionContent::Hooray => 2,
        ReactionContent::Laugh => 3,
        ReactionContent::Rocket => 4,
        ReactionContent::Eyes => 5,
        ReactionContent::Confused => 6,
        ReactionContent::MinusOne => 7,
        _ => usize::MAX,
    }
}

fn reaction_label(content: &ReactionContent) -> &'static str {
    match content {
        ReactionContent::PlusOne => "+1",
        ReactionContent::MinusOne => "-1",
        ReactionContent::Laugh => "laugh",
        ReactionContent::Confused => "confused",
        ReactionContent::Heart => "heart",
        ReactionContent::Hooray => "hooray",
        ReactionContent::Rocket => "rocket",
        ReactionContent::Eyes => "eyes",
        _ => "other",
    }
}

fn reaction_add_options() -> [ReactionContent; 8] {
    [
        ReactionContent::PlusOne,
        ReactionContent::Heart,
        ReactionContent::Hooray,
        ReactionContent::Laugh,
        ReactionContent::Rocket,
        ReactionContent::Eyes,
        ReactionContent::Confused,
        ReactionContent::MinusOne,
    ]
}

fn format_reaction_picker(selected: usize, options: &[ReactionContent]) -> String {
    let mut out = String::new();
    let mut bracket_start = None;
    let mut bracket_end = None;
    const TOTAL_WIDTH: usize = 20;
    for (idx, content) in options.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        let label = reaction_label(content);
        if idx == selected {
            bracket_start = Some(out.len());
            out.push('[');
            out.push_str(label);
            bracket_end = Some(out.len());
            out.push(']');
        } else {
            out.push_str(label);
        }
    }
    if let (Some(start), Some(end)) = (bracket_start, bracket_end) {
        let padding = TOTAL_WIDTH.saturating_sub(end - start + 1);
        let left_padding = padding / 2;
        let left_start = start.saturating_sub(left_padding);
        let right_padding = padding - left_padding;
        let right_end = (end + right_padding).min(out.len());
        return out[left_start..right_end].to_string();
    }
    out
}

fn to_reaction_snapshot<I>(
    reactions: I,
    current_user: &str,
) -> (Vec<(ReactionContent, u64)>, Vec<ReactionContent>)
where
    I: IntoIterator<Item = octocrab::models::reactions::Reaction>,
{
    let mut mine = Vec::new();
    let counts = reactions
        .into_iter()
        .fold(HashMap::new(), |mut acc, reaction| {
            if reaction.user.login.eq_ignore_ascii_case(current_user) {
                mine.push(reaction.content.clone());
            }
            *acc.entry(reaction.content).or_insert(0) += 1_u64;
            acc
        });
    mine.sort_by_key(reaction_order);
    mine.dedup();
    (counts.into_iter().collect::<Vec<_>>(), mine)
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

pub(crate) fn render_markdown_lines(text: &str, width: usize, indent: usize) -> Vec<Line<'static>> {
    render_markdown(text, width, indent).lines
}

fn render_markdown(text: &str, width: usize, indent: usize) -> MarkdownRender {
    let mut renderer = MarkdownRenderer::new(width, indent);
    let options = Options::ENABLE_GFM
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_SUPERSCRIPT
        | Options::ENABLE_SUBSCRIPT
        | Options::ENABLE_MATH;
    let parser = Parser::new_ext(text, options);
    let parser = TextMergeStream::new(parser);
    for event in parser {
        match event {
            Event::Start(tag) => renderer.start_tag(tag),
            Event::End(tag) => renderer.end_tag(tag),
            Event::Text(text) => renderer.text(&text),
            Event::Code(text) => renderer.inline_code(&text),
            Event::InlineMath(text) | Event::DisplayMath(text) => renderer.inline_math(&text),
            Event::SoftBreak => renderer.soft_break(),
            Event::HardBreak => renderer.hard_break(),
            Event::Html(text) | Event::InlineHtml(text) => renderer.text(&text),
            Event::Rule => renderer.rule(),
            Event::TaskListMarker(checked) => renderer.task_list_marker(checked),
            _ => {}
        }
    }
    renderer.finish()
}

struct MarkdownRenderer {
    lines: Vec<Line<'static>>,
    links: Vec<RenderedLink>,
    current_line: Vec<Span<'static>>,
    current_width: usize,
    max_width: usize,
    indent: usize,
    style_stack: Vec<Style>,
    current_style: Style,
    in_block_quote: bool,
    block_quote_style: Option<AdmonitionStyle>,
    block_quote_title_pending: bool,
    in_code_block: bool,
    code_block_lang: Option<String>,
    code_block_buf: String,
    list_prefix: Option<String>,
    pending_space: bool,
    active_link_url: Option<String>,
}

#[derive(Clone, Copy)]
struct AdmonitionStyle {
    marker: &'static str,
    default_title: &'static str,
    border_color: Color,
    title_style: Style,
}

impl AdmonitionStyle {
    fn from_block_quote_kind(kind: BlockQuoteKind) -> Option<Self> {
        match kind {
            BlockQuoteKind::Note => Some(Self {
                marker: "NOTE",
                default_title: "Note",
                border_color: Color::Blue,
                title_style: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            }),
            BlockQuoteKind::Tip => Some(Self {
                marker: "TIP",
                default_title: "Tip",
                border_color: Color::Green,
                title_style: Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            }),
            BlockQuoteKind::Important => Some(Self {
                marker: "IMPORTANT",
                default_title: "Important",
                border_color: Color::Cyan,
                title_style: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            }),
            BlockQuoteKind::Warning => Some(Self {
                marker: "WARNING",
                default_title: "Warning",
                border_color: Color::Yellow,
                title_style: Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            }),
            BlockQuoteKind::Caution => Some(Self {
                marker: "CAUTION",
                default_title: "Caution",
                border_color: Color::Red,
                title_style: Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            }),
        }
    }
}

impl MarkdownRenderer {
    fn new(max_width: usize, indent: usize) -> Self {
        Self {
            lines: Vec::new(),
            links: Vec::new(),
            current_line: Vec::new(),
            current_width: 0,
            max_width: max_width.max(10),
            indent,
            style_stack: Vec::new(),
            current_style: Style::new(),
            in_block_quote: false,
            block_quote_style: None,
            block_quote_title_pending: false,
            in_code_block: false,
            code_block_lang: None,
            code_block_buf: String::new(),
            list_prefix: None,
            pending_space: false,
            active_link_url: None,
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Emphasis => self.push_style(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Superscript | Tag::Subscript => {
                self.push_style(Style::new().add_modifier(Modifier::ITALIC))
            }
            Tag::Link { dest_url, .. } => {
                self.active_link_url = Some(dest_url.to_string());
                self.push_style(
                    Style::new()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Heading { .. } => {
                self.push_style(Style::new().add_modifier(Modifier::BOLD));
            }
            Tag::BlockQuote(kind) => {
                self.flush_line();
                self.in_block_quote = true;
                self.block_quote_style = kind.and_then(AdmonitionStyle::from_block_quote_kind);
                self.block_quote_title_pending = self.block_quote_style.is_some();
            }
            Tag::CodeBlock(kind) => {
                self.ensure_admonition_header();
                self.flush_line();
                self.in_code_block = true;
                self.code_block_lang = code_block_kind_lang(kind);
                self.code_block_buf.clear();
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
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Superscript
            | TagEnd::Subscript
            | TagEnd::Link => {
                if matches!(tag, TagEnd::Link) {
                    self.active_link_url = None;
                }
                self.pop_style();
            }
            TagEnd::Heading(_) => {
                self.pop_style();
                self.flush_line();
            }
            TagEnd::BlockQuote(_) => {
                self.flush_line();
                self.in_block_quote = false;
                self.block_quote_style = None;
                self.block_quote_title_pending = false;
                self.push_blank_line();
            }
            TagEnd::CodeBlock => {
                self.render_code_block();
                self.flush_line();
                self.in_code_block = false;
                self.code_block_lang = None;
                self.code_block_buf.clear();
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
        if self.in_block_quote && self.block_quote_title_pending {
            if let Some(style) = self.block_quote_style
                && let Some(title) = extract_admonition_title(text, style.marker)
            {
                let title = if title.is_empty() {
                    style.default_title
                } else {
                    title
                };
                self.push_admonition_header(title, style);
                self.block_quote_title_pending = false;
                return;
            }
            self.ensure_admonition_header();
        }
        if self.in_code_block {
            self.code_block_text(text);
        } else {
            let style = self.current_style;
            self.push_text(text, style);
        }
    }

    fn inline_code(&mut self, text: &str) {
        self.ensure_admonition_header();
        let style = self
            .current_style
            .patch(Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD));
        self.push_text(text, style);
    }

    fn inline_math(&mut self, text: &str) {
        self.ensure_admonition_header();
        let style = self.current_style.patch(
            Style::new()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::ITALIC),
        );
        self.push_text(text, style);
    }

    fn soft_break(&mut self) {
        self.ensure_admonition_header();
        if self.in_code_block {
            self.code_block_buf.push('\n');
        } else {
            self.pending_space = true;
        }
    }

    fn hard_break(&mut self) {
        self.ensure_admonition_header();
        if self.in_code_block {
            self.code_block_buf.push('\n');
            return;
        }
        self.flush_line();
    }

    fn task_list_marker(&mut self, checked: bool) {
        self.ensure_admonition_header();
        let marker = if checked { "[x] " } else { "[ ] " };
        self.push_text(marker, self.current_style);
    }

    fn rule(&mut self) {
        self.flush_line();
        self.start_line();
        let width = self.max_width.saturating_sub(self.prefix_width()).max(8);
        let bar = "─".repeat(width);
        self.current_line
            .push(Span::styled(bar.clone(), Style::new().fg(Color::DarkGray)));
        self.current_width += display_width(&bar);
        self.flush_line();
        self.push_blank_line();
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
            let space_col = self.current_width;
            self.current_line.push(Span::raw(" "));
            self.current_width += 1;
            self.push_link_segment(" ", space_col, 1);
        }
        self.pending_space = false;

        let link_start_col = self.current_width;
        self.current_line
            .push(Span::styled(word.to_string(), style));
        self.current_width += word_width;
        self.push_link_segment(word, link_start_col, word_width);
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
            let link_start_col = self.current_width;
            let part_width = display_width(part);
            self.current_line
                .push(Span::styled(part.to_string(), style));
            self.current_width += part_width;
            self.push_link_segment(part, link_start_col, part_width);
        }
    }

    fn push_link_segment(&mut self, label: &str, col: usize, width: usize) {
        let Some(url) = self.active_link_url.as_ref() else {
            return;
        };
        if label.is_empty() || width == 0 {
            return;
        }

        let line = self.current_line_index();
        if let Some(last) = self.links.last_mut()
            && last.url == *url
            && last.line == line
            && last.col + last.width == col
        {
            last.label.push_str(label);
            last.width += width;
            return;
        }

        self.links.push(RenderedLink {
            line,
            col,
            label: label.to_string(),
            url: url.clone(),
            width,
        });
    }

    fn current_line_index(&self) -> usize {
        self.lines.len()
    }

    fn code_block_text(&mut self, text: &str) {
        self.code_block_buf.push_str(text);
    }

    fn render_code_block(&mut self) {
        if self.code_block_buf.is_empty() {
            return;
        }

        let code = std::mem::take(&mut self.code_block_buf);
        let assets = syntect_assets();
        let syntax = resolve_syntax(&assets.syntaxes, self.code_block_lang.as_deref());
        let mut highlighter = HighlightLines::new(syntax, &assets.theme);
        let fallback_style = Style::new().light_yellow();

        for raw_line in code.split('\n') {
            self.flush_line();
            self.start_line();
            match highlighter.highlight_line(raw_line, &assets.syntaxes) {
                Ok(regions) => {
                    for (syn_style, fragment) in regions {
                        if fragment.is_empty() {
                            continue;
                        }
                        self.current_line.push(Span::styled(
                            fragment.to_string(),
                            syntect_style_to_ratatui(syn_style),
                        ));
                        self.current_width += display_width(fragment);
                    }
                }
                Err(_) => {
                    if !raw_line.is_empty() {
                        self.current_line
                            .push(Span::styled(raw_line.to_string(), fallback_style));
                        self.current_width += display_width(raw_line);
                    }
                }
            }
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
            let border_style = self
                .block_quote_style
                .map(|s| Style::new().fg(s.border_color))
                .unwrap_or_else(|| Style::new().fg(Color::DarkGray));
            self.current_line.push(Span::styled("│ ", border_style));
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

    fn finish(mut self) -> MarkdownRender {
        self.flush_line();
        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }
        if self.lines.is_empty() {
            self.lines.push(Line::from(vec![Span::raw("")]));
        }
        MarkdownRender {
            lines: self.lines,
            links: self.links,
        }
    }

    fn ensure_admonition_header(&mut self) {
        if !self.block_quote_title_pending {
            return;
        }
        if let Some(style) = self.block_quote_style {
            self.push_admonition_header(style.default_title, style);
        }
        self.block_quote_title_pending = false;
    }

    fn push_admonition_header(&mut self, title: &str, style: AdmonitionStyle) {
        self.flush_line();
        self.start_line();
        self.current_line
            .push(Span::styled(title.to_string(), style.title_style));
        self.current_width += display_width(title);
        self.flush_line();
    }
}

fn extract_admonition_title<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let trimmed = text.trim_start();
    let min_len = marker.len() + 3;
    if trimmed.len() < min_len {
        return None;
    }
    let bytes = trimmed.as_bytes();
    if bytes[0] != b'[' || bytes[1] != b'!' {
        return None;
    }
    let marker_end = 2 + marker.len();
    if bytes.get(marker_end) != Some(&b']') {
        return None;
    }
    if !trimmed[2..marker_end].eq_ignore_ascii_case(marker) {
        return None;
    }
    Some(trimmed[marker_end + 1..].trim())
}

fn code_block_kind_lang(kind: CodeBlockKind<'_>) -> Option<String> {
    match kind {
        CodeBlockKind::Indented => None,
        CodeBlockKind::Fenced(info) => parse_fenced_language(&info).map(|lang| lang.to_lowercase()),
    }
}

fn parse_fenced_language(info: &str) -> Option<&str> {
    let token = info
        .split_ascii_whitespace()
        .next()
        .unwrap_or_default()
        .split(',')
        .next()
        .unwrap_or_default()
        .trim_matches(|c| c == '{' || c == '}');
    let token = token.strip_prefix('.').unwrap_or(token);
    if token.is_empty() { None } else { Some(token) }
}

fn resolve_syntax<'a>(syntaxes: &'a SyntaxSet, lang: Option<&str>) -> &'a SyntaxReference {
    if let Some(lang) = lang {
        if let Some(syntax) = syntaxes.find_syntax_by_token(lang) {
            return syntax;
        }
        if let Some(stripped) = lang.strip_prefix("language-")
            && let Some(syntax) = syntaxes.find_syntax_by_token(stripped)
        {
            return syntax;
        }
        if let Some(syntax) = syntaxes.find_syntax_by_extension(lang) {
            return syntax;
        }
    }
    syntaxes.find_syntax_plain_text()
}

fn syntect_style_to_ratatui(style: syntect::highlighting::Style) -> Style {
    let mut out = Style::new().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::render_markdown;

    #[test]
    fn extracts_link_segments_with_urls() {
        let rendered = render_markdown("Go to [ratatui docs](https://github.com/ratatui/).", 80, 0);

        assert!(!rendered.links.is_empty());
        assert!(
            rendered
                .links
                .iter()
                .all(|link| link.url == "https://github.com/ratatui/")
        );
    }

    #[test]
    fn wraps_long_links_into_multiple_segments() {
        let rendered = render_markdown("[A very long linked label](https://example.com)", 12, 2);

        assert!(rendered.links.len() >= 2);
    }
}
