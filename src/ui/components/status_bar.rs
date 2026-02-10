use rat_widget::statusline_stacked::{SLANT_BL_TR, SLANT_TL_BR, StatusLineStacked};
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use ratatui_macros::span;
use std::sync::atomic::Ordering;

use crate::ui::components::DumbComponent;
use crate::ui::components::issue_list::LOADED_ISSUE_COUNT;
use crate::ui::{AppState, layout::Layout};

pub struct StatusBar {
    repo_label: String,
    user_label: String,
}

impl StatusBar {
    pub fn new(app_state: AppState) -> Self {
        Self {
            repo_label: format!(" {}/{} ", app_state.owner, app_state.repo),
            user_label: format!(" Logged in as {} ", app_state.current_user),
        }
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        let issue_count = LOADED_ISSUE_COUNT.load(Ordering::Relaxed);
        let count_text = format!(" Issues: {} ", issue_count);

        StatusLineStacked::new()
            .start(
                span!(self.user_label.as_str()).style(Style::new().black().on_green()),
                " ",
            )
            .start(span!(self.repo_label.as_str()).style(Style::new()), " ")
            .end(span!(count_text).style(Style::new().black().on_blue()), " ")
            .render(area.status_bar, buf);
    }
}

impl DumbComponent for StatusBar {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }
}
