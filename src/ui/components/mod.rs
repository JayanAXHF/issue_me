use async_trait::async_trait;
use rat_widget::focus::HasFocus;
use ratatui::buffer::Buffer;

use crate::errors::AppError;
use crate::ui::{Action, layout::Layout};
use ratatui::crossterm::event::Event;

pub mod help;
pub mod issue_conversation;
pub mod issue_create;
pub mod issue_detail;
pub mod issue_list;
pub mod label_list;
pub mod search_bar;
pub mod status_bar;
pub mod title_bar;

#[async_trait(?Send)]
pub trait DumbComponent {
    fn render(&mut self, area: Layout, buf: &mut Buffer);
    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        let _ = action_tx;
    }
    async fn handle_event(&mut self, event: Action) -> Result<(), AppError> {
        let _ = event;
        Ok(())
    }
}

#[async_trait(?Send)]
pub trait Component: HasFocus {
    fn render(&mut self, area: Layout, buf: &mut Buffer);
    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        let _ = action_tx;
    }
    async fn handle_event(&mut self, event: Action) -> Result<(), AppError> {
        let _ = event;
        Ok(())
    }
    fn cursor(&self) -> Option<(u16, u16)> {
        None
    }
    fn should_render(&self) -> bool {
        true
    }
    fn is_animating(&self) -> bool {
        false
    }
    fn capture_focus_event(&self, _event: &Event) -> bool {
        false
    }
    #[allow(unused_variables)]
    fn set_index(&mut self, index: usize) {}

    fn set_global_help(&self) {}
}
