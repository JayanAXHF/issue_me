use std::borrow::Cow;

use ratatui::{
    layout::{Constraint, Rect, Size},
    widgets::{Clear, Widget, WidgetRef},
};
use textwrap::wrap;
use tracing::info;

use crate::ui::components::toast::Toast;

const DEFAULT_MAX_TOAST_WIDTH: u16 = 50;

#[derive(Debug)]
pub struct ToastEngine<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    area: Rect,
    default_duration: std::time::Duration,
    tx: Option<tokio::sync::mpsc::Sender<A>>,
    toast_area: Rect,
    current_toast: Option<Toast>,
}

pub struct ToastEngineBuilder<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    area: Rect,
    default_duration: std::time::Duration,
    tx: Option<tokio::sync::mpsc::Sender<A>>,
}

impl<A> ToastEngineBuilder<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    pub fn new(area: Rect) -> Self {
        Self {
            area,
            default_duration: std::time::Duration::from_secs(3),
            tx: None,
        }
    }

    pub fn default_duration(mut self, duration: std::time::Duration) -> Self {
        self.default_duration = duration;
        self
    }

    pub fn action_tx(mut self, tx: tokio::sync::mpsc::Sender<A>) -> Self {
        self.tx = Some(tx);
        self
    }

    pub fn build(self) -> ToastEngine<A> {
        ToastEngine::from_builder(self)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub enum ToastType {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Default, Clone, Copy)]
pub enum ToastPosition {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

#[derive(Debug, Default)]
pub enum ToastConstraint {
    #[default]
    Auto,
    Uniform(Constraint),
    Manual {
        width: Constraint,
        height: Constraint,
    },
}

#[derive(Debug, Clone)]
pub enum ToastMessage {
    Show {
        message: String,
        toast_type: ToastType,
        position: ToastPosition,
    },
    Hide,
}

#[derive(Debug, Default)]
pub struct ToastBuilder {
    message: Cow<'static, str>,
    toast_type: ToastType,
    position: ToastPosition,
    constraint: ToastConstraint,
}

impl<A> ToastEngine<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    pub fn new(
        ToastEngine {
            area,
            default_duration,
            tx,
            ..
        }: Self,
    ) -> Self {
        Self {
            area,
            default_duration,
            tx,
            current_toast: None,
            toast_area: Rect::default(),
        }
    }
    pub fn from_builder(
        ToastEngineBuilder {
            area,
            default_duration,
            tx,
            ..
        }: ToastEngineBuilder<A>,
    ) -> Self {
        Self {
            area,
            default_duration,
            tx,
            current_toast: None,
            toast_area: Rect::default(),
        }
    }

    pub fn show_toast(&mut self, toast: ToastBuilder) {
        let toast_area = calculate_toast_area(&toast, self.area);
        self.toast_area = toast_area;
        let toast = Toast::new(&toast.message, toast.toast_type);
        self.current_toast = Some(toast);
        if let Some(tx) = &self.tx {
            let tx_clone = tx.clone();
            let duration = self.default_duration;
            tokio::spawn(async move {
                tokio::time::sleep(duration).await;
                let _ = tx_clone.send(ToastMessage::Hide.into()).await;
            });
        }
        // Here you would implement the logic to display the toast message
        // based on the toast_type and position. This is a placeholder implementation.
    }

    pub fn hide_toast(&mut self) {
        self.current_toast = None;
    }

    pub fn set_area(&mut self, area: Rect) {
        self.area = area;
    }
}

impl ToastBuilder {
    pub fn new(message: Cow<'static, str>) -> Self {
        Self {
            message,
            toast_type: ToastType::Info,
            position: ToastPosition::TopRight,
            constraint: ToastConstraint::Auto,
        }
    }

    pub fn toast_type(mut self, toast_type: ToastType) -> Self {
        self.toast_type = toast_type;
        self
    }

    pub fn position(mut self, position: ToastPosition) -> Self {
        self.position = position;
        self
    }

    pub fn constraint(mut self, constraint: ToastConstraint) -> Self {
        self.constraint = constraint;
        self
    }
}

fn calculate_toast_area(
    ToastBuilder {
        message,
        position,
        constraint,
        ..
    }: &ToastBuilder,
    area: Rect,
) -> Rect {
    use ToastConstraint::*;
    use ToastPosition::*;
    const PADDING: u16 = 2;

    let width = match constraint {
        Auto => std::cmp::min(DEFAULT_MAX_TOAST_WIDTH, message.len() as u16 + PADDING * 2),
        Uniform(c) => area.centered_horizontally(*c).width,
        Manual { width, .. } => area.centered_horizontally(*width).width,
    };
    let wrapped_text = wrap(message, width as usize);
    let height = match constraint {
        Auto => wrapped_text.len() as u16 + PADDING,
        Uniform(c) => area.centered_vertically(*c).height + PADDING,
        Manual { height, .. } => area.centered_vertically(*height).height + PADDING,
    };
    if let Center = position {
        return area.centered(width.into(), height.into());
    }
    position.calculate_position(area, Size { width, height })
}

impl ToastPosition {
    fn calculate_position(&self, area: Rect, Size { width, height }: Size) -> Rect {
        use ToastPosition::*;
        match self {
            TopLeft => Rect {
                x: area.x,
                y: area.y,
                width,
                height,
            },
            TopRight => Rect {
                x: area.x + area.width.saturating_sub(width),
                y: area.y,
                width,
                height,
            },
            BottomLeft => Rect {
                x: area.x,
                y: area.y + area.height.saturating_sub(height),
                width,
                height,
            },
            BottomRight => Rect {
                x: area.x + area.width.saturating_sub(width),
                y: area.y + area.height.saturating_sub(height),
                width,
                height,
            },
            Center => Rect {
                x: area.x + (area.width.saturating_sub(width)) / 2,
                y: area.y + (area.height.saturating_sub(height)) / 2,
                width,
                height,
            },
        }
    }
}

impl From<ToastType> for ratatui::style::Color {
    fn from(value: ToastType) -> Self {
        use ToastType::*;
        match value {
            Info => Self::Blue,
            Success => Self::Green,
            Warning => Self::Yellow,
            Error => Self::Red,
        }
    }
}

impl<A> WidgetRef for ToastEngine<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    fn render_ref(&self, _area: Rect, buf: &mut ratatui::buffer::Buffer) {
        info!("Rendering toast engine with area: {:?}", self.area);
        if self.current_toast.is_some() {
            Clear.render(self.toast_area, buf);
        }
        self.current_toast.render_ref(self.toast_area, buf);
    }
}
