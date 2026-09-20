//! FIELD shell: ops-pattern command center on the ui-ux-pro-max direction.
//! Dark slate + status green, live-labeled metrics with tick age, pausable
//! refresh, real buttons throughout. Empty states everywhere data can be thin.

use crate::backend::{summarize_event, FieldBackend};
use crate::state::Nav;
use dioxus::prelude::*;

pub const BG: &str = "#0B0E13";
pub const CARD: &str = "#141B26";
pub const MINT: &str = "#2EE6A8";
pub const BLUE: &str = "#4D7CFF";
pub const TEXT: &str = "#E8EEF4";
pub const MUTED: &str = "#8A94A6";
pub const DANGER: &str = "#F0655A";
pub const BORDER: &str = "#263041";

fn card_style() -> String {
    format!(
        "background:{CARD};border:1px solid {BORDER};border-radius:10px;padding:14px;margin:8px;"
    )
}

fn btn_style(primary: bool) -> String {
    let (bg, fg) = if primary {
        (MINT, "#08110C")
    } else {
        ("#1D2534", TEXT)
    };
    format!("background:{bg};color:{fg};border:1px solid {BORDER};border-radius:8px;padding:6px 12px;margin:2px;cursor:pointer;font-weight:600;")
}

#[component]
pub fn App(backend: FieldBackend) -> Element {
    use_context_provider(|| backend.clone());
    let mut nav = use_signal(|| Nav::Overview);
    let mut tick = use_signal(|| 0u64);
    let mut paused = use_signal(|| false);

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if !paused() {
                *tick.write() += 1;
            }
        }
    });

    let tick_n = tick();
    let _paused = paused();
    let backend = use_context::<FieldBackend>();
    let (events, _errors) = backend.event_totals();
    let killed = backend.killed();
    let mut ptt_busy = use_signal(|| false);
    let mut ptt_note = use_signal(String::new);

    rsx! {
        div { style: format!("background:{BG};color:{TEXT};font-family:system-ui,sans-serif;min-height:100vh;display:flex;flex-direction:column;"),
            // Title bar with live presence.
            div { style: format!("display:flex;align-items:center;gap:12px;padding:10px 16px;border-bottom:1px solid {BORDER};"),
                span { style: format!("color:{MINT};font-weight:800;letter-spacing:2px;"), "IGRIS FIELD" }
                span {
                    style: format!("background:{};color:#08110C;border-radius:20px;padding:2px 12px;font-weight:700;font-size:12px;", if killed { DANGER } else { MINT }),
                    if killed { "KILLED" } else { "● LIVE" }
                }
                span { style: format!("color:{MUTED};font-size:12px;"), "tick {tick_n} · events {events} · live data" }
                button {
                    style: btn_style(false),
                    onclick: move |_| paused.set(!paused()),
                    if paused() { "▶ resume live" } else { "⏸ pause live" }
                }
                button {
                    style: btn_style(false),
                    onclick: move |_| {
                        if ptt_busy() {
                            return;
                        }
                        ptt_busy.set(true);
                        ptt_note.set("listening…".into());
                        let b = backend.clone();
                        spawn(async move {
                            let r = tokio::task::spawn_blocking(move || b.record_ptt_blocking(2000)).await;
                            match r {
                                Ok(res) => ptt_note.set(format!("{} samples · {}", res.samples, res.note)),
                                Err(_) => ptt_note.set("capture task failed".into()),
                            }
                            ptt_busy.set(false);
                        });
                    },
                    if ptt_busy() { "LISTENING…" } else { "TALK (2s)" }
                }
                if !ptt_note().is_empty() {
                    span { style: format!("color:{MUTED};font-size:12px;"), "{ptt_note}" }
                }
            }
            div { style: "display:flex;flex:1;min-height:0;",
                // Rail.
                nav { style: format!("display:flex;flex-direction:column;gap:4px;padding:12px;border-right:1px solid {BORDER};min-width:130px;"),
                    for n in Nav::all() {
                        button {
                            key: "{n.label()}",
                            style: format!("text-align:left;background:{};color:{};border:1px solid {};border-radius:8px;padding:8px 10px;cursor:pointer;font-weight:{};",
                                if nav() == n { "#1D2534" } else { "transparent" }, if nav() == n { MINT } else { TEXT }, BORDER, if nav() == n { "700" } else { "400" }),
                            onclick: move |_| nav.set(n),
                            "{n.label()}"
                            {approvals_badge(&backend, n)}
                        }
                    }
                }
                // Center panel.
                main { style: "flex:1;padding:8px;overflow-y:auto;",
                    match nav() {
                        Nav::Overview => rsx! { OverviewPanel { backend: backend.clone() } },
                        Nav::Missions => rsx! { MissionsPanel { backend: backend.clone() } },
                        Nav::Memory => rsx! { MemoryPanel { backend: backend.clone() } },
                        Nav::Approvals => rsx! { ApprovalsPanel { backend: backend.clone() } },
                        Nav::Timeline => rsx! { TimelinePanel { backend: backend.clone() } },
                        Nav::Devices => rsx! { DevicesPanel { backend: backend.clone() } },
                        Nav::Settings => rsx! { SettingsPanel { backend: backend.clone() } },
                    }
                }
            }
            // Status strip.
            footer { style: format!("border-top:1px solid {BORDER};padding:6px 16px;color:{MUTED};font-size:12px;display:flex;gap:16px;"),
                span { "eco :53327 · fastswap :53317" }
                span { "autonomy {backend.autonomy():?}" }
                span { "updated just now (1s live tick)" }
            }
        }
    }
}

fn approvals_badge(backend: &FieldBackend, n: Nav) -> Element {
    if n != Nav::Approvals {
        return rsx! {};
    }
    let count = backend.pending_approvals().len();
    if count == 0 {
        return rsx! {};
    }
    rsx! {
        span { style: format!("background:{DANGER};color:#fff;border-radius:10px;padding:0 8px;margin-left:6px;font-size:12px;"), "{count}" }
    }
}

#[component]
fn OverviewPanel(backend: FieldBackend) -> Element {
    let progress = backend.mission_progress();
    let mem = backend.memory_count();
    let approvals = backend.pending_approvals().len();
    let (events, errors) = backend.event_totals();
    rsx! {
        h2 { "What are we doing?" }
        div { style: "display:flex;flex-wrap:wrap;",
            div { style: card_style(),
                h3 { "Active mission" }
                p { style: format!("font-size:28px;color:{MINT};"), "{progress}%" }
                p { style: format!("color:{MUTED};"), "durable task graph · checkpointed" }
            }
            div { style: card_style(),
                h3 { "Memory" }
                p { style: format!("font-size:28px;color:{BLUE};"), "{mem}" }
                p { style: format!("color:{MUTED};"), "live records · SQLite + FTS" }
            }
            div { style: card_style(),
                h3 { "Approvals" }
                p { style: format!("font-size:28px;color:{};", if approvals > 0 { DANGER } else { MINT }), "{approvals}" }
                p { style: format!("color:{MUTED};"), "pending requests" }
            }
            div { style: card_style(),
                h3 { "Events" }
                p { style: "font-size:28px;", "{events}" }
                p { style: format!("color:{MUTED};"), "errors {errors} · bounded bus" }
            }
        }
    }
}

#[component]
fn MissionsPanel(backend: FieldBackend) -> Element {
    let nodes = backend.mission_nodes();
    let progress = backend.mission_progress();
    rsx! {
        h2 { "Missions" }
        div { style: card_style(),
            p { "Overall {progress}% complete · crash-resumable from checkpoints" }
            button { style: btn_style(true), onclick: move |_| { backend.demo_step(); }, "▶ step next task" }
        }
        if nodes.is_empty() {
            p { style: format!("color:{MUTED};"), "No missions yet." }
        }
        for n in nodes {
            div { style: card_style(), key: "{n.name}",
                p { style: "font-weight:700;", "{n.name}" }
                p { style: format!("color:{};", state_color(&n.state)), "{n.state}" }
                if let Some(cp) = n.checkpoint {
                    p { style: format!("color:{MUTED};font-size:12px;"), "checkpoint: {cp}" }
                }
            }
        }
    }
}

fn state_color(state: &str) -> &str {
    match state {
        "Completed" => MINT,
        "Failed" => DANGER,
        "Running" => BLUE,
        _ => MUTED,
    }
}

#[component]
fn MemoryPanel(backend: FieldBackend) -> Element {
    let mut query = use_signal(String::new);
    let mut note = use_signal(String::new);
    let mut saved = use_signal(String::new);
    let results = backend.memory_search(&query());
    rsx! {
        h2 { "Memory" }
        input {
            style: format!("background:#1D2534;color:{TEXT};border:1px solid {BORDER};border-radius:8px;padding:8px;width:60%;"),
            placeholder: "search memories…",
            value: "{query}",
            oninput: move |e| query.set(e.value()),
        }
        if results.is_empty() {
            p { style: format!("color:{MUTED};"), "No memories match — the store is live, try another query." }
        }
        for r in results {
            div { style: card_style(), key: "{r.id}",
                p { "{r.content}" }
                p { style: format!("color:{MUTED};font-size:12px;"), "{r.kind:?} · {r.source} · conf {r.confidence:.2}" }
            }
        }
        h3 { "Remember" }
        input {
            style: format!("background:#1D2534;color:{TEXT};border:1px solid {BORDER};border-radius:8px;padding:8px;width:60%;"),
            placeholder: "store a note…",
            value: "{note}",
            oninput: move |e| note.set(e.value()),
        }
        button {
            style: btn_style(true),
            onclick: move |_| {
                match backend.memory_put(&note()) {
                    Some(_) => {
                        saved.set("stored".into());
                        note.set(String::new());
                    }
                    None => saved.set("empty note — nothing stored".into()),
                }
            },
            "Remember"
        }
        if !saved().is_empty() {
            p { style: format!("color:{MINT};font-size:12px;"), "{saved}" }
        }
    }
}

#[component]
fn ApprovalsPanel(backend: FieldBackend) -> Element {
    let pending = backend.pending_approvals();
    rsx! {
        h2 { "Approvals" }
        if pending.is_empty() {
            p { style: format!("color:{MUTED};"), "Queue clear — nothing needs you." }
        }
        for a in pending {
            div { style: card_style(), key: "{a.id}",
                p { style: "font-weight:700;", "{a.summary}" }
                p { style: format!("color:{MUTED};font-size:12px;"), "level {a.level:?} · expires {a.expires_at}" }
                button { style: btn_style(true), onclick: { let b = backend.clone(); let id = a.id; move |_| { b.approve(&id); } }, "Approve" }
                button { style: btn_style(false), onclick: { let b = backend.clone(); let id = a.id; move |_| { b.deny(&id); } }, "Deny" }
            }
        }
    }
}

#[component]
fn TimelinePanel(backend: FieldBackend) -> Element {
    // Seed from the telemetry snapshot, then stream live bus events.
    let mut live: Signal<Vec<String>> = use_signal(|| {
        backend
            .timeline(10)
            .iter()
            .map(|e| format!("[{}] {} — {}", e.at.format("%H:%M:%S"), e.kind, e.summary))
            .collect()
    });
    {
        let b = backend.clone();
        use_future(move || {
            let b = b.clone();
            async move {
                let mut rx = b.events.subscribe();
                while let Ok(env) = rx.recv().await {
                    let mut items = live.write();
                    items.push(summarize_event(&env.event));
                    while items.len() > 40 {
                        items.remove(0);
                    }
                }
            }
        });
    }
    let items = live();
    rsx! {
        h2 { "Activity timeline" }
        p { style: format!("color:{MUTED};font-size:12px;"), "live stream · newest last" }
        if items.is_empty() {
            p { style: format!("color:{MUTED};"), "Quiet — nothing recorded yet." }
        }
        for (i, text) in items.iter().enumerate() {
            div { style: format!("border-left:3px solid {MINT};margin:6px;padding:4px 10px;"), key: "{i}{text}",
                p { "{text}" }
            }
        }
    }
}

#[component]
fn DevicesPanel(backend: FieldBackend) -> Element {
    let devices = backend.known_devices();
    let ips = backend.local_ips();
    let mut scanning = use_signal(|| false);
    let mut scan_note = use_signal(String::new);
    rsx! {
        h2 { "Devices" }
        div { style: card_style(),
            p { style: format!("color:{MUTED};"), "local interfaces: {ips.join(\", \")}" }
            p { style: format!("color:{MUTED};"), "eco :53327 · fastswap :53317 · TLS proxy ready" }
            button {
                style: btn_style(true),
                onclick: move |_| {
                    if scanning() {
                        return;
                    }
                    scanning.set(true);
                    scan_note.set("scanning LAN…".into());
                    let b = backend.clone();
                    spawn(async move {
                        let found = b.run_lan_scan().await;
                        scan_note.set(format!("{} device(s) live", found.len()));
                        scanning.set(false);
                    });
                },
                if scanning() { "SCANNING…" } else { "SCAN LAN" }
            }
            if !scan_note().is_empty() {
                p { style: format!("color:{MINT};font-size:12px;"), "{scan_note}" }
            }
        }
        if devices.is_empty() {
            p { style: format!("color:{MUTED};"), "No peers discovered yet — hit SCAN LAN." }
        }
        for d in devices {
            div { style: card_style(), key: "{d.ip}{d.name}",
                p { style: "font-weight:700;", "{d.name}" }
                p { style: format!("color:{MUTED};font-size:12px;"), "{d.platform} · {d.ip}" }
            }
        }
    }
}

#[component]
fn SettingsPanel(backend: FieldBackend) -> Element {
    let killed = backend.killed();
    rsx! {
        h2 { "Settings" }
        div { style: card_style(),
            h3 { "Autonomy" }
            for mode in [igris_policy::Autonomy::Assist, igris_policy::Autonomy::Operate, igris_policy::Autonomy::Autonomous] {
                button {
                    key: "{mode:?}",
                    style: btn_style(backend.autonomy() == mode),
                    onclick: { let b = backend.clone(); move |_| b.set_autonomy(mode) },
                    "{mode:?}"
                }
            }
        }
        div { style: card_style(),
            h3 { "Kill switch" }
            p { style: format!("color:{};", if killed { DANGER } else { MUTED }),
                if killed { "ENGAGED — all execution halted" } else { "released — gates operate normally" } }
            if killed {
                button { style: btn_style(true), onclick: move |_| backend.revive(), "Release kill-switch" }
            } else {
                button { style: btn_style(false), onclick: move |_| backend.kill(), "Engage kill-switch" }
            }
        }
    }
}
