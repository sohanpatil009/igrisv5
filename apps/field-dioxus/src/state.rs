//! FIELD navigation: missions-first, keyboard-reachable tabs.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Overview,
    Missions,
    Memory,
    Approvals,
    Timeline,
    Devices,
    Settings,
}

impl Nav {
    pub fn all() -> [Nav; 7] {
        [
            Nav::Overview,
            Nav::Missions,
            Nav::Memory,
            Nav::Approvals,
            Nav::Timeline,
            Nav::Devices,
            Nav::Settings,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Nav::Overview => "Overview",
            Nav::Missions => "Missions",
            Nav::Memory => "Memory",
            Nav::Approvals => "Approvals",
            Nav::Timeline => "Timeline",
            Nav::Devices => "Devices",
            Nav::Settings => "Settings",
        }
    }
}
