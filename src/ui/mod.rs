pub mod components;
pub mod filter;
pub mod layout;
pub mod macros;
pub mod theme;
pub mod utils;

use crate::{
    app::GITHUB_CLIENT,
    define_cid_map,
    errors::AppError,
    ui::components::{
        Component, DumbComponent,
        issue_conversation::IssueConversation,
        issue_detail::IssuePreview,
        issue_list::{IssueList, MainScreen},
        label_list::LabelList,
        search_bar::TextSearch,
        status_bar::StatusBar,
    },
};
use crossterm::{
    event::{EventStream, KeyEvent, KeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute,
};
use futures::{StreamExt, future::FutureExt};
use octocrab::{
    Page,
    models::{Label, issues::Issue},
};
use rat_widget::{
    event::{HandleEvent, Outcome, Regular},
    focus::{Focus, FocusBuilder, FocusFlag},
};
use ratatui::{
    crossterm,
    prelude::*,
    widgets::{Block, Paragraph, Wrap},
};
use ratatui_macros::line;
use std::{
    collections::HashMap,
    io::stdout,
    sync::{Arc, OnceLock},
};
use termprofile::{DetectorSettings, TermProfile};
use tokio::{select, sync::mpsc::Sender};
use tokio_util::sync::CancellationToken;
use tracing::{info, instrument};

use crate::ui::components::{
    issue_conversation::{CommentView, IssueConversationSeed},
    issue_detail::{IssuePreviewSeed, PrSummary},
};

const TICK_RATE: std::time::Duration = std::time::Duration::from_millis(100);
pub static COLOR_PROFILE: OnceLock<TermProfile> = OnceLock::new();
pub static CIDMAP: OnceLock<HashMap<u8, usize>> = OnceLock::new();
const HELP_TEXT: &str = "
Global Help:\n\
\n\
- Press '1' to focus Search Bar\n\
- Press '2' to focus Issue List\n\
- Press '3' to focus Issue Conversation\n\
- Press '4' to focus Label List\n\
- Press '5' to focus Issue Preview\n\
- Press 'q' or 'Ctrl+C' to quit the application\n\
- Press '?' or 'Ctrl+H' to toggle this help menu\n\
\n\
Navigate through the application using the keyboard shortcuts above. Each component may have its own specific controls once focused.
";

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
            .unwrap();
    }
    let mut terminal = ratatui::init();
    let (action_tx, action_rx) = tokio::sync::mpsc::channel(100);
    let mut app = App::new(
        action_tx,
        action_rx,
        AppState::new(repo, owner, current_user),
    )
    .await?;
    app.run(&mut terminal).await?;
    ratatui::restore();
    Ok(())
}

struct App {
    action_tx: tokio::sync::mpsc::Sender<Action>,
    action_rx: tokio::sync::mpsc::Receiver<Action>,
    focus: Option<Focus>,
    cancel_action: CancellationToken,
    components: Vec<Box<dyn Component>>,
    dumb_components: Vec<Box<dyn DumbComponent>>,
    help: Option<&'static str>,
    in_help: bool,
    last_focused: Option<FocusFlag>,
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

fn focus(state: &mut App) -> &mut Focus {
    focus_noret(state);
    state.focus.as_mut().unwrap()
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
    pub async fn new(
        action_tx: Sender<Action>,
        action_rx: tokio::sync::mpsc::Receiver<Action>,
        state: AppState,
    ) -> Result<Self, AppError> {
        let mut text_search = TextSearch::new(state.clone());
        let status_bar = StatusBar::new(state.clone());
        let mut label_list = LabelList::new(state.clone());
        let mut issue_preview = IssuePreview::new(state.clone());
        let mut issue_conversation = IssueConversation::new(state.clone());
        let issue_handler = GITHUB_CLIENT
            .get()
            .unwrap()
            .inner()
            .issues(state.owner.clone(), state.repo.clone());
        let mut issue_list =
            IssueList::new(issue_handler, state.owner, state.repo, action_tx.clone()).await;

        let comps = define_cid_map!(
             2 -> issue_list,
             3 -> issue_conversation,
             4 -> label_list,
             5 -> issue_preview,
             1 -> text_search, // this needs to be the last one
        )?;
        Ok(Self {
            focus: None,
            in_help: false,
            help: None,
            action_tx,
            action_rx,
            last_focused: None,
            cancel_action: Default::default(),
            components: comps,
            dumb_components: vec![Box::new(status_bar)],
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
        execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
        )?;
        execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
        )?;

        execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
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
            let last = self.components.last().unwrap();
            focus.focus(&**last);
        }
        let ctok = self.cancel_action.clone();
        loop {
            let action = self.action_rx.recv().await;
            if let Some(ref action) = action {
                for component in self.components.iter_mut() {
                    component.handle_event(action.clone()).await;
                    if component.gained_focus() && self.last_focused != Some(component.focus()) {
                        self.last_focused = Some(component.focus());
                        component.set_global_help();
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
                Some(Action::ForceFocusChange) => {
                    let focus = focus(self);
                    let r = focus.next_force();
                    info!(outcome = ?r, "Focus");
                }
                Some(Action::ForceFocusChangeRev) => {
                    let focus = focus(self);
                    let r = focus.prev_force();
                    info!(outcome = ?r, "Focus");
                }
                Some(Action::AppEvent(ref event)) => {
                    self.handle_event(event).await?;
                }
                Some(Action::SetHelp(help)) => {
                    self.help = Some(help);
                }
                Some(Action::Quit) | None => {
                    ctok.cancel();
                    break;
                }
                _ => {}
            }
            if should_draw || matches!(action, Some(Action::ForceRender)) {
                self.draw(terminal)?;
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
        info!(?event, "Handling event");
        if matches!(
            event,
            ct_event!(key press CONTROL-'c') | ct_event!(key press CONTROL-'q')
        ) {
            self.cancel_action.cancel();
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
        let focus = focus(self);
        let outcome = focus.handle(event, Regular);
        info!(outcome = ?outcome, "Focus");
        if let Outcome::Continue = outcome
            && let Key(key) = event
            && !capture_focus
        {
            self.handle_key(key).await?;
        }
        if let Key(key) = event {
            match key.code {
                Char(char)
                    if ('1'..'5').contains(&char)
                        && !self
                            .components
                            .iter()
                            .any(|c| c.should_render() && c.capture_focus_event(event)) =>
                {
                    //SAFETY: char is in range
                    let index: u8 = char.to_digit(10).unwrap().try_into().unwrap();
                    //SAFETY: cid is always in map, and map is static
                    info!("Focusing {}", index);
                    let cid = CIDMAP.get().unwrap().get(&index).unwrap();
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
            let layout = layout::Layout::new(area);
            for component in self.components.iter() {
                if component.should_render()
                    && let Some(p) = component.cursor()
                {
                    f.set_cursor_position(p);
                }
            }
            let buf = f.buffer_mut();
            let title = Paragraph::new(line!["IssueMe"].style(Style::new().bold()))
                .block(Block::bordered().border_type(ratatui::widgets::BorderType::Rounded));
            title.render(layout.title_bar, buf);

            for component in self.components.iter_mut() {
                if component.should_render() {
                    component.render(layout, buf);
                }
            }
            for component in self.dumb_components.iter_mut() {
                component.render(layout, buf);
            }
            if self.in_help {
                let help_text = self.help.unwrap_or(HELP_TEXT);
                let help_component = components::help::HelpComponent::new(
                    Paragraph::new(help_text)
                        .wrap(Wrap { trim: true })
                        .centered(),
                )
                .set_constraints([30, 30])
                .block(
                    Block::bordered()
                        .title("Help")
                        .border_type(ratatui::widgets::BorderType::Rounded),
                );
                help_component.render(area, buf);
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
    NewPage(Arc<Page<Issue>>),
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
    ChangeIssueScreen(MainScreen),
    FinishedLoading,
    ForceFocusChange,
    ForceFocusChangeRev,
    SetHelp(&'static str),
}
