use rat_widget::statusline_stacked::StatusLineStacked;
use ratatui::buffer::Buffer;
use ratatui::style::{Style, Stylize};
use ratatui::widgets::Widget;
use ratatui_macros::{line, span};
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
            user_label: app_state.current_user,
        }
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        let issue_count = LOADED_ISSUE_COUNT.load(Ordering::Relaxed);
        let count_text = format!(" Issues: {} ", issue_count);

        let label = &self.user_label;
        let mut ss = StatusLineStacked::new()
            .start(
                line![
                    span!(" Logged in as").style(Style::new().black().on_green()),
                    span!(" ").style(Style::new().black().on_green()),
                    span!(label).bold().black().on_green(),
                    span!(" ").style(Style::new().black().on_green()),
                ],
                " ",
            )
            .start(span!(self.repo_label.as_str()).style(Style::new()), " ")
            .end(span!(count_text).style(Style::new().black().on_blue()), "")
            .end(
                line![
                    span!("q/<C-q>/<C-c").magenta(),
                    " ",
                    span!(" QUIT ").black().on_magenta().bold()
                ],
                " ",
            )
            .end(
                line![
                    span!("?").magenta(),
                    " ",
                    span!(" HELP ").black().on_magenta().bold()
                ],
                " ",
            );
        #[cfg(target_os = "macos")]
        {
            ss = ss.end(
                line![
                    span!("<C-H>").magenta(),
                    " ",
                    span!(" GLOBAL HELP ").black().on_magenta().bold()
                ],
                " ",
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            ss = ss.end(
                line![
                    span!("<C-h>").magenta(),
                    " ",
                    span!(" GLOBAL HELP ").black().on_magenta().bold()
                ],
                " ",
            );
        }
        ss.render(area.status_bar, buf);
    }
}

impl DumbComponent for StatusBar {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }
}
