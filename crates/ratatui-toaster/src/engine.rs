//! A toast engine for displaying temporary messages in a terminal UI.
//! The `ToastEngine` manages the display of toasts, which are temporary messages that appear on the screen for a short duration. It supports different types of toasts (info, success, warning, error) and allows customization of their position and duration.
//!
//! The `ToastEngine` can be integrated into a terminal UI application using the `ratatui` crate. It provides a builder pattern for creating toasts and handles the timing for automatically hiding toasts after a specified duration.
//! # Tokio Integration
//! The `tokio` feature can be used to tightly integrate the toast engine with applications that use an event based pattern. In your
//! `Action` enum (or equivalent), add a variant that can be converted from `ToastMessage`. For example:
//! ```rust
//! enum Action {
//!     ShowToast(ToastMessage),
//!     // other variants...
//! }
//! ```
//! Then, when you want to show a toast, you can send a `ToastMessage::Show` action through your application's event system, although you do need
//! to handle the `Show` event yourself. When the toast times out, the `ToastEngine` will automatically send a `ToastMessage::Hide` action, which you should also handle to hide the toast.
//! Disable the `tokio` feature if you want to manage the timing of hiding toasts yourself, or if your application does not use an event based pattern.
//!
//! # Animating Toasts
//! The current implementation does not include animations for showing or hiding toasts. However, you can
//! use libraries like [tachyonfx](https://github.com/ratatui/tachyonfx) to add animations to your toasts. You would need to implement the animation logic in your event handling code, triggering animations when showing or hiding toasts based on the `ToastMessage` actions.
use std::borrow::Cow;
#[cfg(not(feature = "tokio"))]
use std::marker::PhantomData;

use ratatui::{
    layout::{Constraint, Rect, Size},
    widgets::{Clear, Widget, WidgetRef},
};
use textwrap::wrap;

use crate::widget::Toast;

const DEFAULT_MAX_TOAST_WIDTH: u16 = 50;

/// A toast engine for displaying temporary messages in a terminal UI.
/// The `ToastEngine` manages the display of toasts, which are temporary messages that appear on the screen for a short duration. It supports different types of toasts (info, success, warning, error) and allows customization of their position and duration.
/// You can call `show_toast` to display a toast, and `hide_toast` to hide the toast. To animate,
/// you can get the area of the toast using `toast_area` and implement your animation logic based on that area. #[derive(Debug)]
/// Caveat: If you're not using the `tokio` feature, create a `ToastEngine<()>`. There is a (hacky) impl to make it work without the `tokio` feature.
pub struct ToastEngine<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    area: Rect,
    default_duration: std::time::Duration,
    #[cfg(feature = "tokio")]
    tx: Option<tokio::sync::mpsc::Sender<A>>,
    #[cfg(not(feature = "tokio"))]
    tx: Option<PhantomData<A>>,
    toast_area: Rect,
    current_toast: Option<Toast>,
}

/// A builder for creating a `ToastEngine`. It allows you to set the default duration for toasts, and an optional channel sender for sending toast messages (if using the `tokio` feature).
pub struct ToastEngineBuilder<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    area: Rect,
    default_duration: std::time::Duration,
    #[cfg(feature = "tokio")]
    tx: Option<tokio::sync::mpsc::Sender<A>>,
    #[cfg(not(feature = "tokio"))]
    tx: Option<PhantomData<A>>,
}

impl<A> ToastEngineBuilder<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    /// Creates a new `ToastEngineBuilder` with the specified area for displaying toasts. The default duration for toasts is set to 3 seconds, and no channel sender is configured by default.
    pub fn new(area: Rect) -> Self {
        Self {
            area,
            default_duration: std::time::Duration::from_secs(3),
            tx: None,
        }
    }

    /// Sets the default duration for toasts. This duration will be used when showing a toast if no specific duration is provided.
    pub fn default_duration(mut self, duration: std::time::Duration) -> Self {
        self.default_duration = duration;
        self
    }

    /// Configures a channel sender for sending toast messages. This is used when the `tokio` feature is enabled to allow the `ToastEngine` to send messages to hide toasts after the duration expires.
    #[cfg(feature = "tokio")]
    pub fn action_tx(mut self, tx: tokio::sync::mpsc::Sender<A>) -> Self {
        self.tx = Some(tx);
        self
    }

    /// Builds the `ToastEngine` using the configured settings. This method consumes the builder and returns a new instance of `ToastEngine`.
    pub fn build(self) -> ToastEngine<A> {
        ToastEngine::from_builder(self)
    }
}

/// The type of toast to display. This enum defines the different types of toasts that can be shown, such as informational messages, success messages, warnings, and errors. Each variant can be styled differently when rendered.
#[derive(Debug, Default, Clone, Copy)]
pub enum ToastType {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

/// The position on the screen where the toast should be displayed. This enum defines various positions for toasts, including top-left, top-right, bottom-left, bottom-right, and center. The `ToastEngine` uses this information to calculate the appropriate area for rendering the toast based on the specified position.
#[derive(Debug, Default, Clone, Copy)]
pub enum ToastPosition {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

/// The constraint for the toast's size. This enum defines how the size of the toast should be determined. The `Auto` variant allows the toast to automatically size itself based on the message content, while the `Uniform` and `Manual` variants allow for more specific control over the width and height of the toast.
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

/// The messages that can be sent to the `ToastEngine` to control the display of toasts. The `Show` variant contains the message to display, the type of toast, and its position, while the `Hide` variant indicates that any currently displayed toast should be hidden.
///
///NOTE: You do have to handle the events yourself. Usually, its as simple as matching on the `ToastMessage` in your event loop and calling the appropriate methods on the `ToastEngine` to show or hide toasts based on the received messages.
#[derive(Debug, Clone)]
pub enum ToastMessage {
    Show {
        message: String,
        toast_type: ToastType,
        position: ToastPosition,
    },
    Hide,
}

/// A builder for creating a toast message. This struct allows you to specify the message content, type, position, and size constraints for a toast before showing it using the `ToastEngine`. The builder pattern provides a convenient way to configure the properties of a toast in a fluent manner.
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
    /// Creates a new `ToastEngine`. Consider using the `ToastEngineBuilder` instead.
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

    /// Creates a new `ToastEngine` from a `ToastEngineBuilder`. This method takes the configuration from the builder and initializes the `ToastEngine` accordingly. It sets up the area for displaying toasts, the default duration for toasts, and any channel sender if provided (when using the `tokio` feature).
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

    /// Shows a toast message using the provided `ToastBuilder`. This method calculates the area for the toast based on the message content and the specified position, creates a new `Toast` instance, and sets it as the current toast to be rendered. If the `tokio` feature is enabled and a channel sender is configured, it also spawns a task to automatically hide the toast after the default duration.
    pub fn show_toast(&mut self, toast: ToastBuilder) {
        let toast_area = calculate_toast_area(&toast, self.area);
        self.toast_area = toast_area;
        let toast = Toast::new(&toast.message, toast.toast_type);
        self.current_toast = Some(toast);
        #[cfg(feature = "tokio")]
        if let Some(tx) = &self.tx {
            let tx_clone = tx.clone();
            let duration = self.default_duration;
            tokio::spawn(async move {
                tokio::time::sleep(duration).await;
                let _ = tx_clone.send(ToastMessage::Hide.into()).await;
            });
        }
    }

    /// Get the area where the toast will be rendered.
    pub fn toast_area(&self) -> Rect {
        self.toast_area
    }

    /// Whether a toast is currently being displayed.
    pub fn has_toast(&self) -> bool {
        self.current_toast.is_some()
    }

    /// Hides the currently displayed toast, if any. This method sets the current toast to `None`, which will cause it to no longer be rendered on the screen.
    pub fn hide_toast(&mut self) {
        self.current_toast = None;
    }

    /// Sets the area for the toast engine. This method allows you to update the area where toasts will be displayed, which can be useful if the layout of your terminal UI changes and you need to adjust the toast display area accordingly.
    pub fn set_area(&mut self, area: Rect) {
        self.area = area;
    }
}

impl ToastBuilder {
    /// Create a new instance of a `ToastBuilder`
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
        if self.current_toast.is_some() {
            Clear.render(self.toast_area, buf);
        }
        self.current_toast.render_ref(self.toast_area, buf);
    }
}

impl<A> Widget for &ToastEngine<A>
where
    A: From<ToastMessage> + Send + 'static,
{
    fn render(self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        self.render_ref(area, buf);
    }
}

impl From<ToastMessage> for () {
    fn from(_value: ToastMessage) -> Self {}
}
