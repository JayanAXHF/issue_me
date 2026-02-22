//! A simple example of using the toast engine.
//! This example demonstrates how to create a toast engine, show a toast message, and hide it after a short delay.
//!NOTE: This example is a bit contrived and is meant to demonstrate the basic usage of the toast engine. In a real application, you would likely want to integrate the toast engine into your application's event loop and handle showing and hiding toasts based on user actions or other events.
use std::thread;

use anyhow::Result;
use ratatui::{
    prelude::*,
    widgets::{Block, Paragraph},
};
use ratatui_toaster::{ToastBuilder, ToastEngine, ToastEngineBuilder};

fn main() -> Result<()> {
    let mut terminal = ratatui::init();
    let mut engine: ToastEngine<()> = ToastEngineBuilder::new(Rect::default()).build();
    terminal.draw(|f| {
        let area = f.area();
        let buf = f.buffer_mut();
        let content = Paragraph::new("This is a simple toast example.").block(Block::bordered());
        content.render(area, buf);
    })?;
    thread::sleep(std::time::Duration::from_secs(1));
    terminal.draw(|f| {
        let area = f.area();
        let buf = f.buffer_mut();
        let content = Paragraph::new("This is a simple toast example.").block(Block::bordered());
        content.render(area, buf);
        engine.set_area(area);
        engine.show_toast(ToastBuilder::new("Hello, World!".into()));
        engine.render(area, buf);
    })?;
    thread::sleep(std::time::Duration::from_secs(1));
    terminal.draw(|f| {
        let area = f.area();
        let buf = f.buffer_mut();
        let content = Paragraph::new("This is a simple toast example.").block(Block::bordered());
        content.render(area, buf);
        engine.set_area(area);
        engine.hide_toast();
    })?;
    thread::sleep(std::time::Duration::from_secs(1));
    ratatui::restore();
    Ok(())
}
