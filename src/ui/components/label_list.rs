use std::str::FromStr;

use async_trait::async_trait;
use octocrab::models::Label;
use rat_widget::{
    event::HandleEvent,
    focus::HasFocus,
    list::{ListState, selection::RowSelection},
};
use ratatui::{
    buffer::Buffer,
    style::{Color, Stylize},
    widgets::{Block, ListItem, StatefulWidget},
};
use ratatui_macros::{line, span};

use crate::ui::{COLOR_PROFILE, components::Component, layout::Layout, utils::get_border_style};

const MARKER: &str = ratatui::symbols::marker::DOT;

#[derive(Debug, Default)]
pub struct LabelList {
    state: ListState<RowSelection>,
    labels: Vec<LabelListItem>,
    action_tx: Option<tokio::sync::mpsc::Sender<crate::ui::Action>>,
}

#[derive(Debug, Clone)]
struct LabelListItem(Label);

impl From<Label> for LabelListItem {
    fn from(value: Label) -> Self {
        Self(value)
    }
}

impl std::ops::Deref for LabelListItem {
    type Target = Label;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&LabelListItem> for ListItem<'_> {
    fn from(value: &LabelListItem) -> Self {
        let rgb = &value.0.color;
        let mut c = Color::from_str(&format!("#{}", rgb)).unwrap();
        if let Some(profile) = COLOR_PROFILE.get() {
            let adapted = profile.adapt_color(c);
            if let Some(adapted) = adapted {
                c = adapted;
            }
        }
        let line = line![span!("{} {}", MARKER, value.0.name).fg(c)];
        ListItem::new(line)
    }
}

impl LabelList {
    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(get_border_style(&self.state))
            .title("Labels");
        let list = rat_widget::list::List::<RowSelection>::new(
            self.labels.iter().map(Into::<ListItem>::into),
        )
        .block(block);
        list.render(area.label_list, buf, &mut self.state);
    }
}

#[async_trait(?Send)]
impl Component for LabelList {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }
    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<crate::ui::Action>) {
        self.action_tx = Some(action_tx);
    }
    async fn handle_event(&mut self, event: crate::ui::Action) {
        match event {
            crate::ui::Action::AppEvent(ref event) => {
                self.state.handle(event, rat_widget::event::Regular);
            }
            crate::ui::Action::ChangeLabels(labels) => {
                self.labels = labels
                    .into_iter()
                    .map(Into::<LabelListItem>::into)
                    .collect();
                self.action_tx
                    .as_ref()
                    .unwrap()
                    .send(crate::ui::Action::Render)
                    .await
                    .unwrap();
            }
            _ => {}
        }
    }
}
impl HasFocus for LabelList {
    fn build(&self, builder: &mut rat_widget::focus::FocusBuilder) {
        let tag = builder.start(self);
        builder.widget(&self.state);
        builder.end(tag);
    }
    fn area(&self) -> ratatui::layout::Rect {
        self.state.area()
    }
    fn focus(&self) -> rat_widget::focus::FocusFlag {
        self.state.focus()
    }
}
