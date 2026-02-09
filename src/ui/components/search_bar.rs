use async_trait::async_trait;
use rat_cursor::HasScreenCursor;
use rat_widget::{
    choice::{Choice, ChoiceState},
    event::{HandleEvent, Popup, Regular, ct_event},
    focus::{HasFocus, impl_has_focus},
    popup::Placement,
};
use ratatui::{
    buffer::Buffer,
    style::Style,
    widgets::{Block, BorderType, StatefulWidget, Widget},
};
use throbber_widgets_tui::ThrobberState;
use tracing::info;
use tracing::instrument;

use crate::{
    app::GITHUB_CLIENT,
    ui::{
        Action, AppState,
        components::Component,
        layout::Layout,
        utils::{get_border_style, get_loader_area},
    },
};

const OPTIONS: [&str; 3] = ["Open", "Closed", "All"];

pub struct TextSearch {
    search_state: rat_widget::text_input::TextInputState,
    label_state: rat_widget::text_input::TextInputState,
    cstate: ChoiceState,
    state: State,
    action_tx: Option<tokio::sync::mpsc::Sender<Action>>,
    loader_state: ThrobberState,
    repo: String,
    owner: String,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    Loading,
    #[default]
    Loaded,
}

impl TextSearch {
    pub fn new(AppState { repo, owner, .. }: AppState) -> Self {
        Self {
            repo,
            owner,
            search_state: Default::default(),
            label_state: Default::default(),
            loader_state: Default::default(),
            state: Default::default(),
            cstate: Default::default(),
            action_tx: None,
        }
    }

    fn render_w(&mut self, layout: Layout, buf: &mut Buffer) {
        let contents = (1..).zip(OPTIONS).collect::<Vec<_>>();
        let text_input = rat_widget::text_input::TextInput::new().block(
            Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(get_border_style(&self.search_state))
                .title("Search"),
        );
        let label = rat_widget::text_input::TextInput::new().block(
            Block::bordered()
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(get_border_style(&self.label_state))
                .title("Search Labels"),
        );
        let (widget, popup) = Choice::new()
            .items(contents)
            .popup_placement(Placement::Below)
            .popup_style(Style::default())
            .focus_style(Style::default())
            .select_style(Style::default())
            .button_style(Style::default())
            .style(Style::default())
            .select_marker('>')
            .into_widgets();
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&self.cstate));
        let binner = block.inner(layout.status_dropdown);
        block.render(layout.status_dropdown, buf);
        popup.render(layout.status_dropdown, buf, &mut self.cstate);
        widget.render(binner, buf, &mut self.cstate);
        text_input.render(layout.text_search, buf, &mut self.search_state);
        label.render(layout.label_search, buf, &mut self.label_state);
        if self.state == State::Loading {
            let area = get_loader_area(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .inner(layout.text_search),
            );
            let full = throbber_widgets_tui::Throbber::default()
                .label("Loading")
                .style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan))
                .throbber_set(throbber_widgets_tui::BRAILLE_SIX_DOUBLE)
                .use_type(throbber_widgets_tui::WhichUse::Spin);
            StatefulWidget::render(full, area, buf, &mut self.loader_state);
        }
    }

    #[instrument(skip(self, action_tx))]
    async fn execute_search(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        let mut search = self.search_state.text().to_string();
        let label = self.label_state.text();
        if !label.is_empty() {
            let label_q = label.split(';').map(|s| format!("label:{s}"));
            search.push(' ');
            search.push_str(&label_q.collect::<Vec<_>>().join(" "));
        }
        let status = self.cstate.selected();
        info!(status, "Searching with status");
        if let Some(status) = status
            && status != 2
        {
            search.push_str(&format!(" is:{}", OPTIONS[status].to_lowercase()));
        }
        let repo_q = format!("repo:{}/{}", self.owner, self.repo);
        search.push(' ');
        search.push_str(&repo_q);
        search.push_str(" is:issue");
        info!(search, "Searching with query");
        self.state = State::Loading;
        tokio::spawn(async move {
            let page = GITHUB_CLIENT
                .get()
                .unwrap()
                .search()
                .issues_and_pull_requests(&search)
                .sort("created")
                .order("desc")
                .send()
                .await?;
            action_tx.send(Action::NewPage(Box::new(page))).await?;
            action_tx.send(Action::FinishedLoading).await?;
            Ok::<(), crate::errors::AppError>(())
        });
    }

    ///NOTE: Its named this way to not conflict with the `has_focus`
    /// fn from the impl_has_focus! macro
    fn self_is_focused(&self) -> bool {
        self.search_state.is_focused() || self.label_state.is_focused() || self.cstate.is_focused()
    }
}

impl_has_focus!(search_state, label_state, cstate for TextSearch);

#[async_trait(?Send)]
impl Component for TextSearch {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render_w(area, buf);
    }

    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        self.action_tx = Some(action_tx);
    }
    async fn handle_event(&mut self, event: Action) {
        match event {
            Action::AppEvent(ref event) => {
                if self.self_is_focused() {
                    match event {
                        ct_event!(keycode press Enter) => {
                            if let Some(action_tx) = self.action_tx.clone() {
                                self.execute_search(action_tx).await;
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                self.label_state.handle(event, Regular);
                self.search_state.handle(event, Regular);
                self.cstate.handle(event, Popup);
            }
            Action::FinishedLoading => {
                self.state = State::Loaded;
            }
            Action::Tick => {
                if self.state == State::Loading {
                    self.loader_state.calc_next();
                }
            }
            _ => {}
        }
    }
    fn cursor(&self) -> Option<(u16, u16)> {
        self.search_state
            .screen_cursor()
            .or(self.label_state.screen_cursor())
            .or(self.cstate.screen_cursor())
    }
}
