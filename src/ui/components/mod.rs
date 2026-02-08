use async_trait::async_trait;
use rat_widget::focus::HasFocus;
use ratatui::buffer::Buffer;

use crate::ui::{Action, layout::Layout};

pub mod issue_detail;
pub mod issue_list;
pub mod label_filter;
pub mod label_list;
pub mod search_bar;
pub mod status_bar;

pub trait DumbComponent {
    fn render(&mut self, area: Layout, buf: &mut Buffer);
}

#[async_trait(?Send)]
pub trait Component: HasFocus {
    fn render(&mut self, area: Layout, buf: &mut Buffer);
    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        let _ = action_tx;
    }
    async fn handle_event(&mut self, event: Action) {
        let _ = event;
    }
}
