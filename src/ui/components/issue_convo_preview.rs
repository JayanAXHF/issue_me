use async_trait::async_trait;
use crossterm::event;
use rat_widget::{
    event::{HandleEvent, Regular, ct_event},
    focus::{FocusBuilder, FocusFlag, HasFocus, Navigation},
    paragraph::ParagraphState,
};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{
        Block, Borders, List as TuiList, ListItem, ListState as TuiListState, Padding,
        StatefulWidget, Widget,
    },
};
use std::sync::{Arc, RwLock};
use textwrap::wrap;

use crate::{
    errors::AppError,
    ui::{
        Action,
        components::{
            Component,
            help::HelpElementKind,
            issue_conversation::render_markdown,
            issue_detail::IssuePreviewSeed,
            issue_list::{MainScreen, build_issue_list_item, build_issue_list_lines},
        },
        issue_data::{IssueId, UiIssuePool},
        layout::Layout,
        utils::get_border_style,
    },
};

pub const HELP: &[HelpElementKind] = &[
    crate::help_text!("Issue Conversation Preview Help"),
    crate::help_text!("* marks the issue currently open in details"),
    crate::help_keybind!("Up/Down", "select nearby issue"),
    crate::help_keybind!("Enter", "open selected issue"),
    crate::help_keybind!("Tab", "move focus forward"),
    crate::help_keybind!("Shift+Tab / Esc", "move focus back"),
];

pub struct IssueConvoPreview {
    action_tx: Option<tokio::sync::mpsc::Sender<Action>>,
    issue_pool: Arc<RwLock<UiIssuePool>>,
    body: Option<Arc<str>>,
    issue_ids: Vec<IssueId>,
    open_number: Option<u64>,
    selected_number: Option<u64>,
    screen: MainScreen,
    area: Rect,
    paragraph_state: ParagraphState,
    list_state: TuiListState,
    index: usize,
    focus: FocusFlag,
}

impl IssueConvoPreview {
    pub fn new(issue_pool: Arc<RwLock<UiIssuePool>>) -> Self {
        Self {
            action_tx: None,
            issue_pool,
            body: None,
            issue_ids: Vec::new(),
            open_number: None,
            selected_number: None,
            screen: MainScreen::List,
            area: Rect::default(),
            paragraph_state: ParagraphState::default(),
            list_state: TuiListState::default(),
            index: 0,
            focus: FocusFlag::new().with_name("issue_convo_preview"),
        }
    }

    pub fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.area = area.mini_convo_preview;
        match self.screen {
            MainScreen::List => self.render_body_preview(area.mini_convo_preview, buf),
            MainScreen::Details => self.render_issue_list_preview(area.mini_convo_preview, buf),
            MainScreen::DetailsFullscreen | MainScreen::CreateIssue => {}
        }
    }

    fn render_body_preview(&mut self, area: Rect, buf: &mut Buffer) {
        let block_template = Block::default()
            .borders(Borders::LEFT | Borders::BOTTOM)
            .border_style(get_border_style(&self.paragraph_state));

        let Some(ref body) = self.body else {
            let para =
                ratatui::widgets::Paragraph::new("Select an issue to preview the conversation")
                    .block(
                        block_template
                            .title(format!("[{}] Issue Conversation", self.index))
                            .merge_borders(ratatui::symbols::merge::MergeStrategy::Exact),
                    );
            para.render(area, buf);
            return;
        };
        let body_str = wrap(body, area.width.saturating_sub(2) as usize).join("\n");
        let rendered = render_markdown(&body_str, area.width.saturating_sub(2).into(), 2).lines;
        let para = rat_widget::paragraph::Paragraph::new(rendered).block(
            Block::default()
                .borders(Borders::LEFT | Borders::BOTTOM)
                .title(format!("[{}] Issue Body", self.index))
                .merge_borders(ratatui::symbols::merge::MergeStrategy::Exact)
                .border_style(get_border_style(&self.paragraph_state)),
        );
        para.render(area, buf, &mut self.paragraph_state);
    }

    fn render_issue_list_preview(&mut self, area: Rect, buf: &mut Buffer) {
        let block = Block::default()
            .borders(Borders::LEFT | Borders::BOTTOM)
            .padding(Padding::horizontal(1))
            .title(format!("[{}] Nearby Issues", self.index))
            .merge_borders(ratatui::symbols::merge::MergeStrategy::Exact)
            .border_style(get_border_style(&self.paragraph_state));

        if self.issue_ids.is_empty() {
            let para = ratatui::widgets::Paragraph::new("No nearby issues available.").block(block);
            para.render(area, buf);
            return;
        }

        let items = {
            let pool = self.issue_pool.read().expect("issue pool lock poisoned");
            self.issue_ids
                .iter()
                .map(|issue_id| {
                    let issue = pool.get_issue(*issue_id);
                    if Some(issue.number) == self.open_number {
                        let mut lines = build_issue_list_lines(issue, &pool, false, false);
                        if let Some(first_line) = lines.first_mut() {
                            first_line.spans.insert(
                                0,
                                Span::styled(
                                    "* ",
                                    Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
                                ),
                            );
                        }
                        ListItem::new(lines)
                    } else {
                        build_issue_list_item(issue, &pool, false, false)
                    }
                })
                .collect::<Vec<_>>()
        };

        let selected = self.selected_number.and_then(|number| {
            let pool = self.issue_pool.read().expect("issue pool lock poisoned");
            self.issue_ids
                .iter()
                .position(|issue_id| pool.get_issue(*issue_id).number == number)
        });
        self.list_state.select(selected);

        let list = TuiList::new(items)
            .block(block)
            .highlight_style(Style::new().add_modifier(Modifier::BOLD | Modifier::REVERSED));
        StatefulWidget::render(list, area, buf, &mut self.list_state);
    }

    fn selected_issue_id(&self) -> Option<IssueId> {
        let selected = self.list_state.selected()?;
        self.issue_ids.get(selected).copied()
    }

    fn sync_selected_issue(&mut self) {
        let selected = self.selected_number.and_then(|number| {
            let pool = self.issue_pool.read().expect("issue pool lock poisoned");
            self.issue_ids
                .iter()
                .position(|issue_id| pool.get_issue(*issue_id).number == number)
        });
        self.list_state.select(selected);
    }

    async fn open_selected_issue(&mut self) -> Result<(), AppError> {
        let Some(issue_id) = self.selected_issue_id() else {
            return Ok(());
        };
        let Some(action_tx) = self.action_tx.clone() else {
            return Ok(());
        };

        let (number, labels, preview_seed, conversation_seed) = {
            let pool = self.issue_pool.read().expect("issue pool lock poisoned");
            let issue = pool.get_issue(issue_id);
            (
                issue.number,
                issue.labels.clone(),
                IssuePreviewSeed::from_ui_issue(issue, &pool),
                crate::ui::components::issue_conversation::IssueConversationSeed::from_ui_issue(
                    issue, &pool,
                ),
            )
        };

        self.open_number = Some(number);
        self.selected_number = Some(number);
        self.sync_selected_issue();
        action_tx
            .send(Action::SelectedIssue { number, labels })
            .await?;
        action_tx
            .send(Action::SelectedIssuePreview { seed: preview_seed })
            .await?;
        action_tx
            .send(Action::IssueListPreviewUpdated {
                issue_ids: self.issue_ids.clone(),
                selected_number: number,
            })
            .await?;
        action_tx
            .send(Action::EnterIssueDetails {
                seed: conversation_seed,
            })
            .await?;
        Ok(())
    }
}

#[async_trait(?Send)]
impl Component for IssueConvoPreview {
    fn render(&mut self, area: Layout, buf: &mut Buffer) {
        self.render(area, buf);
    }

    fn register_action_tx(&mut self, action_tx: tokio::sync::mpsc::Sender<Action>) {
        self.action_tx = Some(action_tx);
    }

    async fn handle_event(&mut self, event: Action) -> Result<(), AppError> {
        match event {
            Action::AppEvent(ref event) => {
                if self.screen == MainScreen::List {
                    self.paragraph_state.handle(event, Regular);
                } else if self.screen == MainScreen::Details && self.paragraph_state.is_focused() {
                    match event {
                        ct_event!(keycode press Up) => {
                            self.list_state.select_previous();
                            self.selected_number = self.selected_issue_id().map(|issue_id| {
                                let pool =
                                    self.issue_pool.read().expect("issue pool lock poisoned");
                                pool.get_issue(issue_id).number
                            });
                        }
                        ct_event!(keycode press Down) => {
                            self.list_state.select_next();
                            self.selected_number = self.selected_issue_id().map(|issue_id| {
                                let pool =
                                    self.issue_pool.read().expect("issue pool lock poisoned");
                                pool.get_issue(issue_id).number
                            });
                        }
                        ct_event!(keycode press Enter) => {
                            self.open_selected_issue().await?;
                        }
                        ct_event!(keycode press Tab) => {
                            if let Some(action_tx) = self.action_tx.as_ref() {
                                action_tx.send(Action::ForceFocusChange).await?;
                            }
                        }
                        ct_event!(keycode press SHIFT-BackTab) | ct_event!(keycode press Esc) => {
                            if let Some(action_tx) = self.action_tx.as_ref() {
                                action_tx.send(Action::ForceFocusChangeRev).await?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Action::ChangeIssueBodyPreview(body) => {
                self.body = Some(body);
            }
            Action::IssueListPreviewUpdated {
                issue_ids,
                selected_number,
            } => {
                self.issue_ids = issue_ids;
                self.open_number = Some(selected_number);
                self.selected_number = Some(selected_number);
                self.sync_selected_issue();
            }
            Action::ChangeIssueScreen(screen) => {
                self.screen = screen;
                if screen != MainScreen::Details {
                    self.paragraph_state.focus.set(false);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn should_render(&self) -> bool {
        true
    }

    fn is_animating(&self) -> bool {
        false
    }

    fn set_index(&mut self, index: usize) {
        self.index = index;
    }

    fn set_global_help(&self) {
        if let Some(action_tx) = &self.action_tx {
            let _ = action_tx.try_send(Action::SetHelp(HELP));
        }
    }

    fn capture_focus_event(&self, event: &event::Event) -> bool {
        if self.screen != MainScreen::Details || !self.paragraph_state.is_focused() {
            return false;
        }

        match event {
            event::Event::Key(key) => matches!(
                key.code,
                event::KeyCode::Up
                    | event::KeyCode::Down
                    | event::KeyCode::Enter
                    | event::KeyCode::Tab
                    | event::KeyCode::BackTab
                    | event::KeyCode::Esc
            ),
            _ => false,
        }
    }
}

impl HasFocus for IssueConvoPreview {
    fn build(&self, builder: &mut FocusBuilder) {
        let tag = builder.start(self);
        builder.widget(&self.paragraph_state);
        builder.end(tag);
    }

    fn focus(&self) -> FocusFlag {
        self.focus.clone()
    }

    fn area(&self) -> Rect {
        self.area
    }

    fn navigable(&self) -> Navigation {
        if self.screen == MainScreen::Details {
            Navigation::Regular
        } else {
            Navigation::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testing::{DummyDataConfig, dummy_ui_data_with};
    use octocrab::models::Label;
    use ratatui::{buffer::Buffer, layout::Rect};
    use tokio::sync::mpsc;

    fn buffer_text(buf: &Buffer) -> String {
        let area = buf.area;
        (area.top()..area.bottom())
            .map(|y| {
                (area.left()..area.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_body_preview_in_list_mode() {
        let data = dummy_ui_data_with(DummyDataConfig {
            issue_count: 3,
            ..DummyDataConfig::default()
        });
        let pool = Arc::new(RwLock::new(data.pool));
        let mut preview = IssueConvoPreview::new(pool);
        preview.body = Some(Arc::<str>::from("hello from preview body"));

        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        preview.render(Layout::fullscreen(Rect::new(0, 0, 80, 24)), &mut buf);

        let text = buffer_text(&buf);
        assert!(text.contains("Issue Body"));
        assert!(text.contains("hello from preview body"));
    }

    #[test]
    fn renders_nearby_issues_in_details_mode() {
        let data = dummy_ui_data_with(DummyDataConfig {
            issue_count: 4,
            ..DummyDataConfig::default()
        });
        let selected_id = data.issue_ids[1];
        let open_number = data.issue_numbers[1];
        let selected_number = data.issue_numbers[2];
        let pool = Arc::new(RwLock::new(data.pool));
        let mut preview = IssueConvoPreview::new(pool);
        preview.screen = MainScreen::Details;
        preview.issue_ids = data.issue_ids.clone();
        preview.open_number = Some(open_number);
        preview.selected_number = Some(selected_number);
        preview.sync_selected_issue();

        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        preview.render(Layout::fullscreen(Rect::new(0, 0, 80, 24)), &mut buf);

        let text = buffer_text(&buf);
        assert!(text.contains("Nearby Issues"));
        assert!(text.contains(&format!("#{open_number}")));
        assert!(text.contains(&format!("#{selected_number}")));

        let pool = preview.issue_pool.read().expect("issue pool lock poisoned");
        let open_title = pool.resolve_str(pool.get_issue(selected_id).title);
        let selected_title = pool.resolve_str(pool.get_issue(data.issue_ids[2]).title);
        assert!(text.contains(&format!("* {open_title}")));
        assert!(!text.contains(&format!("* {selected_title}")));
    }

    #[test]
    fn renders_nothing_in_fullscreen_mode() {
        let data = dummy_ui_data_with(DummyDataConfig::default());
        let pool = Arc::new(RwLock::new(data.pool));
        let mut preview = IssueConvoPreview::new(pool);
        preview.screen = MainScreen::DetailsFullscreen;

        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        preview.render(Layout::fullscreen(Rect::new(0, 0, 80, 24)), &mut buf);

        let text = buffer_text(&buf);
        assert!(text.trim().is_empty());
    }

    #[tokio::test]
    async fn opens_selected_issue_from_preview() {
        let data = dummy_ui_data_with(DummyDataConfig {
            issue_count: 4,
            ..DummyDataConfig::default()
        });
        let selected_id = data.issue_ids[1];
        let selected_number = data.issue_numbers[1];
        let expected_author = data
            .preview_seeds
            .get(&selected_id)
            .expect("preview seed should exist")
            .author
            .clone();
        let expected_labels: Vec<Label> = {
            let issue = data.pool.get_issue(selected_id);
            issue.labels.clone()
        };
        let pool = Arc::new(RwLock::new(data.pool));
        let mut preview = IssueConvoPreview::new(pool);
        let (tx, mut rx) = mpsc::channel(8);
        preview.register_action_tx(tx);
        preview.screen = MainScreen::Details;
        preview.issue_ids = data.issue_ids.clone();
        preview.selected_number = Some(selected_number);
        preview.sync_selected_issue();

        preview
            .open_selected_issue()
            .await
            .expect("open should succeed");

        match rx.recv().await.expect("selected issue action") {
            Action::SelectedIssue { number, labels } => {
                assert_eq!(number, selected_number);
                assert_eq!(labels, expected_labels);
            }
            other => panic!("unexpected action: {other:?}"),
        }

        match rx.recv().await.expect("selected issue preview action") {
            Action::SelectedIssuePreview { seed } => {
                assert_eq!(seed.number, selected_number);
                assert_eq!(seed.author, expected_author);
            }
            other => panic!("unexpected action: {other:?}"),
        }

        match rx.recv().await.expect("preview refresh action") {
            Action::IssueListPreviewUpdated {
                issue_ids,
                selected_number: number,
            } => {
                assert_eq!(number, selected_number);
                assert_eq!(issue_ids, data.issue_ids);
            }
            other => panic!("unexpected action: {other:?}"),
        }

        match rx.recv().await.expect("enter details action") {
            Action::EnterIssueDetails { seed } => {
                assert_eq!(seed.number, selected_number);
            }
            other => panic!("unexpected action: {other:?}"),
        }
    }
}
