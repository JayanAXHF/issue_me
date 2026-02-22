# ratatui-toaster

`ratatui-toaster` is an extremely lightweight (under 300 LoC) library for displaying toast notifications in terminal applications built with Ratatui. It provides a simple API for showing transient messages to users, enhancing the user experience without overwhelming them with information.

![Made with VHS](https://vhs.charm.sh/vhs-4S0DJEx8HiykhJvTRnuQDE.gif)

### Features

- Display toast notifications with customizable messages and durations
- Support for different toast types (e.g., success, error, info)
- Easy integration with Ratatui applications
- Minimalistic design to keep the focus on the content

### Installation

```bash
cargo add ratatui-toaster
```

### `tokio` feature

To use the `tokio` feature, add it to your `Cargo.toml`:

```toml
[dependencies]
ratatui-toaster = { version = "0.1", features = ["tokio"] }
```

# Tokio Integration

The `tokio` feature can be used to tightly integrate the toast engine with applications that use an event based pattern. In your
`Action` enum (or equivalent), add a variant that can be converted from `ToastMessage`. For example:

```rust
enum Action {
    ShowToast(ToastMessage),
    // other variants...
}
```

Then, when you want to show a toast, you can send a `ToastMessage::Show` action through your application's event system, although you do need
to handle the `Show` event yourself. When the toast times out, the `ToastEngine` will automatically send a `ToastMessage::Hide` action, which you should also handle to hide the toast.
Disable the `tokio` feature if you want to manage the timing of hiding toasts yourself, or if your application does not use an event based pattern.
