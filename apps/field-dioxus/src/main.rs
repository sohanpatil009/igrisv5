//! IGRIS FIELD — Dioxus native command center.
//! Boots live backends (never simulated) and opens the mission-first shell.

mod backend;
mod state;
mod ui;

pub use backend::FieldBackend;
use dioxus::prelude::*;
use std::sync::OnceLock;

static BACKEND: OnceLock<FieldBackend> = OnceLock::new();

fn root() -> Element {
    let backend = BACKEND.get().expect("backend booted in main").clone();
    rsx! { ui::App { backend: backend } }
}

fn main() {
    BACKEND.set(FieldBackend::boot()).expect("boot once");
    // Title only: browser args stay out (env-provided) after they proved
    // hostile to WebView2 creation when baked into the window config.
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_window(dioxus::desktop::WindowBuilder::new().with_title("IGRIS FIELD")),
        )
        .launch(root);
}
