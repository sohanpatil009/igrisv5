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
    dioxus::launch(root);
}
