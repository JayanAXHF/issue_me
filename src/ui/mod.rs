pub mod components;
pub mod layout;
pub mod macros;
pub mod theme;
pub mod utils;
pub mod widgets;

use crate::{
    app::GITHUB_CLIENT,
    define_cid_map,
    errors::{AppError, Result},
    ui::components::{
        Component, DumbComponent,
        help::HelpElementKind,
        issue_conversation::IssueConversation,
        issue_create::IssueCreate,
        issue_detail::IssuePreview,
        issue_list::{IssueList, MainScreen},
        label_list::LabelList,
        search_bar::TextSearch,
        status_bar::StatusBar,
        title_bar::TitleBar,
        toast::{ToastBuilder, ToastEngineBuilder, ToastMessage},
    },
};
use crossterm::{
    event::{
        DisableBracketedPaste, EnableBracketedPaste, EventStream, KeyEvent,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
};
use futures::{StreamExt, future::FutureExt};
use octocrab::{
    Page,
    models::{Label, issues::Issue, reactions::ReactionContent},
};
use rat_widget::{
    event::{HandleEvent, Outcome, Regular},
    focus::{Focus, FocusBuilder, FocusFlag},
};
use ratatui::{
    crossterm,
    prelude::*,
    widgets::{Block, Clear, Padding, Paragraph, WidgetRef, Wrap},
};
use std::{
    collections::HashMap,
    fmt::Display,
    io::stdout,
    sync::{Arc, OnceLock},
};
use termprofile::{DetectorSettings, TermProfile};
use tokio::{select, sync::mpsc::Sender};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, instrument, trace};

use anyhow::anyhow;

use crate::ui::components::{
    issue_conversation::{CommentView, IssueConversationSeed, TimelineEventView},
    issue_detail::{IssuePreviewSeed, PrSummary},
};

const TICK_RATE: std::time::Duration = std::time::Duration::from_millis(100);
pub static COLOR_PROFILE: OnceLock<TermProfile> = OnceLock::new();
pub static CIDMAP: OnceLock<HashMap<u8, usize>> = OnceLock::new();
const HELP_TEXT: &[HelpElementKind] = &[
    crate::help_text!("Global Help"),
    crate::help_text!(""),
    crate::help_keybind!("1", "focus Search Bar"),
    crate::help_keybind!("2", "focus Issue List"),
    crate::help_keybind!("3", "focus Issue Conversation"),
    crate::help_keybind!("4", "focus Label List"),
    crate::help_keybind!("5", "focus Issue Create"),
    crate::help_keybind!("q / Ctrl+C", "quit the application"),
    crate::help_keybind!("? / Ctrl+H", "toggle help menu"),
    crate::help_text!(""),
    crate::help_text!(
        "Navigate with the focus keys above. Components may have additional controls."
    ),
];

pub async fn run(
    AppState {
        repo,
        owner,
        current_user,
    }: AppState,
) -> Result<(), AppError> {
    if COLOR_PROFILE.get().is_none() {
        COLOR_PROFILE
            .set(TermProfile::detect(&stdout(), DetectorSettings::default()))
            .map_err(|_| AppError::ErrorSettingGlobal("color profile"))?;
    }
    let mut terminal = ratatui::init();
    setup_more_panic_hooks();
    let (action_tx, action_rx) = tokio::sync::mpsc::channel(100);
    let mut app = App::new(
        action_tx,
        action_rx,
        AppState::new(repo, owner, current_user),
    )
    .await?;
    let run_result = app.run(&mut terminal).await;
    ratatui::restore();
    finish_teardown()?;
    run_result
}

struct App {
    action_tx: tokio::sync::mpsc::Sender<Action>,
    action_rx: tokio::sync::mpsc::Receiver<Action>,
    toast_engine: Option<components::toast::ToastEngine<Action>>,
    focus: Option<Focus>,
    cancel_action: CancellationToken,
    components: Vec<Box<dyn Component>>,
    dumb_components: Vec<Box<dyn DumbComponent>>,
    help: Option<&'static [HelpElementKind]>,
    in_help: bool,
    in_editor: bool,
    current_screen: MainScreen,
    last_focused: Option<FocusFlag>,
    last_event_error: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct AppState {
    repo: String,
    owner: String,
    current_user: String,
}

impl AppState {
    pub fn new(repo: String, owner: String, current_user: String) -> Self {
        Self {
            repo,
            owner,
            current_user,
        }
    }
}

fn focus(state: &mut App) -> Result<&mut Focus, AppError> {
    focus_noret(state);
    state
        .focus
        .as_mut()
        .ok_or_else(|| AppError::Other(anyhow!("focus state was not initialized")))
}

fn focus_noret(state: &mut App) {
    let mut f = FocusBuilder::new(state.focus.take());
    for component in state.components.iter() {
        if component.should_render() {
            f.widget(component.as_ref());
        }
    }
    state.focus = Some(f.build());
}

impl App {
    fn capture_error(&mut self, err: impl Display) {
        let message = err.to_string();
        error!(error = %message, "captured ui error");
        self.last_event_error = Some(message);
    }

    pub async fn new(
        action_tx: Sender<Action>,
        action_rx: tokio::sync::mpsc::Receiver<Action>,
        state: AppState,
    ) -> Result<Self, AppError> {
        let mut text_search = TextSearch::new(state.clone());
        let status_bar = StatusBar::new(state.clone());
        let mut label_list = LabelList::new(state.clone());
        let issue_preview = IssuePreview::new(state.clone());
        let mut issue_conversation = IssueConversation::new(state.clone());
        let mut issue_create = IssueCreate::new(state.clone());
        let issue_handler = GITHUB_CLIENT
            .get()
            .ok_or_else(|| AppError::Other(anyhow!("github client is not initialized")))?
            .inner()
            .issues(state.owner.clone(), state.repo.clone());
        let mut issue_list = IssueList::new(
            issue_handler,
            state.owner.clone(),
            state.repo.clone(),
            action_tx.clone(),
        )
        .await;

        let comps = define_cid_map!(
             2 -> issue_list,
             3 -> issue_conversation,
             5 -> issue_create,
             4 -> label_list,
             1 -> text_search, // this needs to be the last one
        )?;
        Ok(Self {
            focus: None,
            toast_engine: None,
            in_help: false,
            in_editor: false,
            current_screen: MainScreen::default(),
            help: None,
            action_tx,
            action_rx,
            last_focused: None,
            last_event_error: None,
            cancel_action: Default::default(),
            components: comps,
            dumb_components: vec![
                Box::new(status_bar),
                Box::new(issue_preview),
                Box::new(TitleBar),
            ],
        })
    }
    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<impl std::io::Write>>,
    ) -> Result<(), AppError> {
        let ctok = self.cancel_action.clone();
        let action_tx = self.action_tx.clone();
        for component in self.components.iter_mut() {
            component.register_action_tx(action_tx.clone());
        }

        if let Err(err) = setup_terminal() {
            self.capture_error(err);
        }

        tokio::spawn(async move {
            let mut tick_interval = tokio::time::interval(TICK_RATE);
            let mut event_stream = EventStream::new();

            loop {
                let event = select! {
                    _ = ctok.cancelled() => break,
                    _ = tick_interval.tick() => Action::Tick,
                    kevent = event_stream.next().fuse() => {
                        match kevent {
                            Some(Ok(kevent)) => Action::AppEvent(kevent),
                            Some(Err(..)) => Action::None,
                            None => break,
                        }
                    }
                };
                if action_tx.send(event).await.is_err() {
                    break;
                }
            }
            Ok::<(), AppError>(())
        });
        focus_noret(self);
        if let Some(ref mut focus) = self.focus {
            if let Some(last) = self.components.last() {
                focus.focus(&**last);
            } else {
                self.capture_error(anyhow!("no components available to focus"));
            }
        }
        let ctok = self.cancel_action.clone();
        let builder = ToastEngineBuilder::new(Rect::default()).action_tx(self.action_tx.clone());
        self.toast_engine = Some(builder.build());
        loop {
            let action = self.action_rx.recv().await;
            let mut should_draw_error_popup = false;
            let mut full_redraw = false;
            if let Some(ref action) = action {
                if let Action::EditorModeChanged(enabled) = action {
                    self.in_editor = *enabled;
                    if *enabled {
                        continue;
                    }
                    full_redraw = true;
                }
                if self.in_editor && matches!(action, Action::Tick | Action::AppEvent(_)) {
                    continue;
                }
                for component in self.components.iter_mut() {
                    if let Err(err) = component.handle_event(action.clone()).await {
                        let message = err.to_string();
                        error!(error = %message, "captured ui error");
                        self.last_event_error = Some(message);
                        should_draw_error_popup = true;
                    }
                    if component.gained_focus() && self.last_focused != Some(component.focus()) {
                        self.last_focused = Some(component.focus());
                        component.set_global_help();
                    }
                }
                for component in self.dumb_components.iter_mut() {
                    if let Err(err) = component.handle_event(action.clone()).await {
                        let message = err.to_string();
                        error!(error = %message, "captured ui error");
                        self.last_event_error = Some(message);
                        should_draw_error_popup = true;
                    }
                }
            }
            let should_draw = match &action {
                Some(Action::Tick) => self.has_animated_components(),
                Some(Action::None) => false,
                Some(Action::Quit) | None => false,
                _ => true,
            };
            match action {
                Some(Action::None) | Some(Action::Tick) => {}
                Some(Action::ToastAction(ref toast_action)) => match toast_action {
                    ToastMessage::Show {
                        message,
                        toast_type,
                        position,
                    } => {
                        if let Some(ref mut toast_engine) = self.toast_engine {
                            toast_engine.show_toast(
                                ToastBuilder::new(message.clone().into())
                                    .toast_type(*toast_type)
                                    .position(*position),
                            );
                        }
                    }
                    ToastMessage::Hide => {
                        if let Some(ref mut toast_engine) = self.toast_engine {
                            toast_engine.hide_toast();
                        }
                    }
                },
                Some(Action::ForceFocusChange) => match focus(self) {
                    Ok(focus) => {
                        let r = focus.next_force();
                        trace!(outcome = ?r, "Focus");
                    }
                    Err(err) => {
                        self.capture_error(err);
                        should_draw_error_popup = true;
                    }
                },
                Some(Action::ForceFocusChangeRev) => match focus(self) {
                    Ok(focus) => {
                        let r = focus.prev_force();
                        trace!(outcome = ?r, "Focus");
                    }
                    Err(err) => {
                        self.capture_error(err);
                        should_draw_error_popup = true;
                    }
                },
                Some(Action::AppEvent(ref event)) => {
                    info!(?event, "Received app event");
                    if let Err(err) = self.handle_event(event).await {
                        self.capture_error(err);
                        should_draw_error_popup = true;
                    }
                }
                Some(Action::SetHelp(help)) => {
                    self.help = Some(help);
                }
                Some(Action::EditorModeChanged(enabled)) => {
                    self.in_editor = enabled;
                }
                Some(Action::ChangeIssueScreen(screen)) => {
                    self.current_screen = screen;
                    focus_noret(self);
                }
                Some(Action::Quit) | None => {
                    ctok.cancel();

                    break;
                }
                _ => {}
            }
            if !self.in_editor
                && (should_draw
                    || matches!(action, Some(Action::ForceRender))
                    || should_draw_error_popup)
            {
                if full_redraw && let Err(err) = terminal.clear() {
                    self.capture_error(err);
                }
                if let Err(err) = self.draw(terminal) {
                    self.capture_error(err);
                }
            }
            if self.cancel_action.is_cancelled() {
                break;
            }
        }

        Ok(())
    }
    #[instrument(skip(self))]
    async fn handle_event(&mut self, event: &crossterm::event::Event) -> Result<(), AppError> {
        use crossterm::event::Event::Key;
        use crossterm::event::KeyCode::*;
        use rat_widget::event::ct_event;
        trace!(?event, "Handling event");
        if matches!(
            event,
            ct_event!(key press CONTROL-'c') | ct_event!(key press CONTROL-'q')
        ) {
            self.cancel_action.cancel();
            return Ok(());
        }
        if self.last_event_error.is_some() {
            if matches!(
                event,
                ct_event!(keycode press Esc) | ct_event!(keycode press Enter)
            ) {
                self.last_event_error = None;
            }
            return Ok(());
        }
        if matches!(event, ct_event!(key press CONTROL-'h')) {
            self.in_help = !self.in_help;
            self.help = Some(HELP_TEXT);
            return Ok(());
        }
        if self.in_help && matches!(event, ct_event!(keycode press Esc)) {
            self.in_help = false;
            return Ok(());
        }

        let capture_focus = self
            .components
            .iter()
            .any(|c| c.should_render() && c.capture_focus_event(event));
        let focus = focus(self)?;
        let outcome = focus.handle(event, Regular);
        trace!(outcome = ?outcome, "Focus");
        if let Outcome::Continue = outcome
            && let Key(key) = event
            && !capture_focus
        {
            self.handle_key(key).await?;
        }
        if let Key(key) = event {
            match key.code {
                Char(char)
                    if ('1'..='6').contains(&char)
                        && !self
                            .components
                            .iter()
                            .any(|c| c.should_render() && c.capture_focus_event(event)) =>
                {
                    //SAFETY: char is in range
                    let index: u8 = char
                        .to_digit(10)
                        .ok_or_else(|| {
                            AppError::Other(anyhow!("failed to parse focus shortcut from key"))
                        })?
                        .try_into()
                        .map_err(|_| {
                            AppError::Other(anyhow!("focus shortcut is out of expected range"))
                        })?;
                    //SAFETY: cid is always in map, and map is static
                    trace!("Focusing {}", index);
                    let cid_map = CIDMAP
                        .get()
                        .ok_or_else(|| AppError::ErrorSettingGlobal("component id map"))?;
                    let cid = cid_map.get(&index).ok_or_else(|| {
                        AppError::Other(anyhow!("component id {index} not found in focus map"))
                    })?;
                    //SAFETY: cid is in map, and map is static
                    let component = unsafe { self.components.get_unchecked(*cid) };

                    if let Some(f) = self.focus.as_mut() {
                        f.focus(component.as_ref());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    async fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> Result<(), AppError> {
        use crossterm::event::KeyCode::*;
        if matches!(key.code, Char('q'))
            | matches!(
                key,
                KeyEvent {
                    code: Char('c' | 'q'),
                    modifiers: crossterm::event::KeyModifiers::CONTROL,
                    ..
                }
            )
        {
            self.cancel_action.cancel();
        }
        if matches!(key.code, Char('?')) {
            self.in_help = !self.in_help;
        }

        Ok(())
    }

    fn has_animated_components(&self) -> bool {
        self.components
            .iter()
            .any(|component| component.should_render() && component.is_animating())
    }

    fn draw(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<impl std::io::Write>>,
    ) -> Result<(), AppError> {
        terminal.draw(|f| {
            let area = f.area();
            let fullscreen = self.current_screen == MainScreen::DetailsFullscreen;
            let layout = if fullscreen {
                layout::Layout::fullscreen(area)
            } else {
                layout::Layout::new(area)
            };
            for component in self.components.iter() {
                if component.should_render()
                    && let Some(p) = component.cursor()
                {
                    f.set_cursor_position(p);
                }
            }
            let buf = f.buffer_mut();

            for component in self.components.iter_mut() {
                if component.should_render() {
                    component.render(layout, buf);
                }
            }
            if !fullscreen {
                for component in self.dumb_components.iter_mut() {
                    component.render(layout, buf);
                }
            }
            if self.in_help {
                let help_text = self.help.unwrap_or(HELP_TEXT);
                let help_component = components::help::HelpComponent::new(help_text)
                    .set_constraint(30)
                    .block(
                        Block::bordered()
                            .title("Help")
                            .padding(Padding::horizontal(2))
                            .border_type(ratatui::widgets::BorderType::Rounded),
                    );
                help_component.render(area, buf);
            }
            if let Some(err) = self.last_event_error.as_ref() {
                let popup_area = area.centered(Constraint::Percentage(60), Constraint::Length(5));
                Clear.render(popup_area, buf);
                let popup = Paragraph::new(err.as_str())
                    .wrap(Wrap { trim: false })
                    .block(
                        Block::bordered()
                            .title("Error")
                            .title_bottom("Esc/Enter: dismiss")
                            .padding(Padding::horizontal(1))
                            .border_type(ratatui::widgets::BorderType::Rounded),
                    );
                popup.render(popup_area, buf);
            }
            if let Some(ref mut toast_engine) = self.toast_engine {
                toast_engine.set_area(area);
                toast_engine.render_ref(area, f.buffer_mut());
            }
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Action {
    None,
    Tick,
    Quit,
    AppEvent(crossterm::event::Event),
    RefreshIssueList,
    NewPage(Arc<Page<Issue>>, MergeStrategy),
    ForceRender,
    SelectedIssue {
        number: u64,
        labels: Vec<Label>,
    },
    SelectedIssuePreview {
        seed: IssuePreviewSeed,
    },
    IssuePreviewLoaded {
        number: u64,
        open_prs: Vec<PrSummary>,
    },
    IssuePreviewError {
        number: u64,
        message: String,
    },
    EnterIssueDetails {
        seed: IssueConversationSeed,
    },
    IssueCommentsLoaded {
        number: u64,
        comments: Vec<CommentView>,
    },
    IssueTimelineLoaded {
        number: u64,
        events: Vec<TimelineEventView>,
    },
    IssueTimelineError {
        number: u64,
        message: String,
    },
    IssueReactionsLoaded {
        reactions: HashMap<u64, Vec<(ReactionContent, u64)>>,
        own_reactions: HashMap<u64, Vec<ReactionContent>>,
    },
    IssueReactionEditError {
        comment_id: u64,
        message: String,
    },
    IssueCommentPosted {
        number: u64,
        comment: CommentView,
    },
    IssueCommentsError {
        number: u64,
        message: String,
    },
    IssueCommentPostError {
        number: u64,
        message: String,
    },
    IssueCommentEditFinished {
        issue_number: u64,
        comment_id: u64,
        result: std::result::Result<String, String>,
    },
    IssueCommentPatched {
        issue_number: u64,
        comment: CommentView,
    },
    EnterIssueCreate,
    IssueCreateSuccess {
        issue: Box<Issue>,
    },
    IssueCreateError {
        message: String,
    },
    IssueCloseSuccess {
        issue: Box<Issue>,
    },
    IssueCloseError {
        number: u64,
        message: String,
    },
    IssueLabelsUpdated {
        number: u64,
        labels: Vec<Label>,
    },
    LabelMissing {
        name: String,
    },
    LabelEditError {
        message: String,
    },
    LabelSearchPageAppend {
        request_id: u64,
        items: Vec<Label>,
        scanned: u32,
        matched: u32,
    },
    LabelSearchFinished {
        request_id: u64,
        scanned: u32,
        matched: u32,
    },
    LabelSearchError {
        request_id: u64,
        message: String,
    },
    ChangeIssueScreen(MainScreen),
    FinishedLoading,
    ForceFocusChange,
    ForceFocusChangeRev,
    SetHelp(&'static [HelpElementKind]),
    EditorModeChanged(bool),
    ToastAction(crate::ui::components::toast::ToastMessage),
}

impl From<crate::ui::components::toast::ToastMessage> for Action {
    fn from(value: crate::ui::components::toast::ToastMessage) -> Self {
        Self::ToastAction(value)
    }
}

#[derive(Debug, Clone)]
pub enum MergeStrategy {
    Append,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseIssueReason {
    Completed,
    NotPlanned,
    Duplicate,
}

impl CloseIssueReason {
    pub const ALL: [Self; 3] = [Self::Completed, Self::NotPlanned, Self::Duplicate];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Completed => "Completed",
            Self::NotPlanned => "Not planned",
            Self::Duplicate => "Duplicate",
        }
    }

    pub const fn to_octocrab(self) -> octocrab::models::issues::IssueStateReason {
        match self {
            Self::Completed => octocrab::models::issues::IssueStateReason::Completed,
            Self::NotPlanned => octocrab::models::issues::IssueStateReason::NotPlanned,
            Self::Duplicate => octocrab::models::issues::IssueStateReason::Duplicate,
        }
    }
}

fn finish_teardown() -> Result<()> {
    let mut stdout = stdout();
    execute!(stdout, PopKeyboardEnhancementFlags)?;
    execute!(stdout, DisableBracketedPaste)?;

    Ok(())
}

fn setup_terminal() -> Result<()> {
    let mut stdout = stdout();
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
    )?;
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
    )?;
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
    )?;
    execute!(stdout, EnableBracketedPaste)?;

    Ok(())
}

fn setup_more_panic_hooks() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // we want to log the panic with tracing, but also preserve the default panic behavior of printing to stderr and aborting
        tracing::error!(panic_info = ?info, "Panic occurred");
        let _ = finish_teardown();
        hook(info);
    }));
}

fn toast_action(
    message: impl Into<String>,
    toast_type: crate::ui::components::toast::ToastType,
) -> Action {
    use crate::ui::components::toast::ToastPosition::TopRight;
    Action::ToastAction(crate::ui::components::toast::ToastMessage::Show {
        message: message.into(),
        toast_type,
        position: TopRight,
    })
}
