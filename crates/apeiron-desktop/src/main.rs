//! The desktop app of Apeiron (DESKTOP.md §1): starts the local server
//! inside itself and shows its pages in a window of the system's web view.
//! The address with the token never leaves this process.

#![forbid(unsafe_code)]
// No console window next to the app on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::error::Error;

use apeiron_server::{Local, Options};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::{NewWindowResponse, WebViewBuilder};

fn main() {
    if let Err(e) = run() {
        eprintln!("apeiron-desktop: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let server = Local::start(Options::with_memory())?;
    let url = server.url().to_owned();
    // Everything the pages may open is on the server's own address.
    let origin = url.split('#').next().unwrap_or_default().to_owned();

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Apeiron")
        .with_inner_size(LogicalSize::new(1100.0, 800.0))
        .with_min_inner_size(LogicalSize::new(480.0, 400.0))
        .build(&event_loop)?;
    let builder = WebViewBuilder::new()
        .with_url(&url)
        .with_navigation_handler(move |to| to.starts_with(&origin))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny);

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let webview = builder.build(&window)?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window
            .default_vbox()
            .ok_or("the window has no GTK container")?;
        builder.build_gtk(vbox)?
    };

    let alive = (server, webview);
    event_loop.run(move |event, _, control_flow| {
        // The server and the web view live as long as the window.
        let _ = &alive;
        *control_flow = ControlFlow::Wait;
        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}
