use std::{io::stdout, sync::OnceLock, time::Duration};

use crate::{
    app::GITHUB_CLIENT,
    errors::AppError,
    ui::components::{
        Component, DumbComponent, issue_list::IssueList, label_list::LabelList,
        search_bar::TextSearch, status_bar::StatusBar,
    },
};
use crossterm::event::EventStream;
use futures::{StreamExt, future::FutureExt};
use octocrab::{
    Page,
    models::{Label, issues::Issue},
};
use rat_widget::{
    event::{HandleEvent, Regular},
    focus::{Focus, FocusBuilder},
};
use ratatui::{crossterm, prelude::*, widgets::Block};
use termprofile::{DetectorSettings, TermProfile};
use tokio::{select, sync::mpsc::Sender};
use tokio_util::sync::CancellationToken;

const TICK_RATE: std::time::Duration = std::time::Duration::from_millis(100);
const FPS: usize = 60;
pub static COLOR_PROFILE: OnceLock<TermProfile> = OnceLock::new();

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
    let mut f = FocusBuilder::new(state.focus.take());
    for component in state.components.iter() {
        f.widget(component.as_ref());
    }
    state.focus = Some(f.build());
    state.focus.as_mut().unwrap()
}

impl App {
    pub async fn new(
        action_tx: Sender<Action>,
        action_rx: tokio::sync::mpsc::Receiver<Action>,
        state: AppState,
    ) -> Result<Self, AppError> {
        let text_search = TextSearch::default();
        let status_bar = StatusBar::new(state.clone());
        let label_list = LabelList::default();
        let issue_handler = GITHUB_CLIENT
            .get()
            .unwrap()
            .inner()
            .issues(state.owner.clone(), state.repo.clone());
        let issue_list =
            IssueList::new(issue_handler, state.owner, state.repo, action_tx.clone()).await;
        Ok(Self {
            focus: None,
            action_tx,
            action_rx,
            cancel_action: Default::default(),
            components: vec![
                Box::new(text_search),
                Box::new(issue_list),
                Box::new(label_list),
            ],
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

        tokio::spawn(async move {
            let mut tick_interval = tokio::time::interval(TICK_RATE);
            let mut frame_interval =
                tokio::time::interval(Duration::from_secs_f64(1.0 / FPS as f64));
            let mut event_stream = EventStream::new();

            loop {
                let event = select! {
                    _ = ctok.cancelled() => break,
                    _ = tick_interval.tick() => Action::Tick,
                    _ = frame_interval.tick() => {
                        Action::Render
                    },
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

        let ctok = self.cancel_action.clone();
        loop {
            let action = self.action_rx.recv().await;
            if let Some(ref action) = action {
                for component in self.components.iter_mut() {
                    component.handle_event(action.clone()).await;
                }
            }
            match action {
                Some(Action::None) => {}
                Some(Action::Tick) => {
                    terminal.draw(|f| {
                        let layout = layout::Layout::new(f.area());
                        let buf = f.buffer_mut();
                        let areas = layout.areas();
                        for area in areas {
                            let w = Block::bordered()
                                .border_type(ratatui::widgets::BorderType::Rounded);
                            w.render(area, buf);
                        }
                        for component in self.components.iter_mut() {
                            component.render(layout, buf);
                        }
                        for component in self.dumb_components.iter_mut() {
                            component.render(layout, buf);
                        }
                    })?;
                }
                Some(Action::Render) => {}
                Some(Action::AppEvent(event)) => {
                    self.handle_event(event).await?;
                }
                Some(Action::Quit) | None => {
                    ctok.cancel();
                    break;
                }
                _ => {}
            }
            if self.cancel_action.is_cancelled() {
                break;
            }
        }

        Ok(())
    }
    async fn handle_event(&mut self, event: crossterm::event::Event) -> Result<(), AppError> {
        let focus = focus(self);
        focus.handle(&event, Regular);
        if let crossterm::event::Event::Key(key) = event {
            self.handle_key(key).await?;
        }
        Ok(())
    }
    async fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Result<(), AppError> {
        match key.code {
            crossterm::event::KeyCode::Char('q') => {
                self.cancel_action.cancel();
            }
            _ => {}
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Action {
    None,
    Tick,
    Render,
    Quit,
    AppEvent(crossterm::event::Event),
    NewPage(Box<Page<Issue>>),
    ChangeLabels(Vec<Label>),
}

pub mod components;
pub mod layout;
pub mod theme;
