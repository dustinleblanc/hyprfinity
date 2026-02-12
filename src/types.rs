use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct Monitor {
    pub(crate) name: Option<String>,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) x: i32,
    pub(crate) y: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct AutoTuneProfile {
    pub(crate) render_scale: f32,
    pub(crate) reason: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Client {
    pub(crate) pid: i32,
    #[serde(default)]
    pub(crate) address: Option<String>,
    #[serde(default)]
    pub(crate) at: Option<[i32; 2]>,
    #[serde(default)]
    pub(crate) size: Option<[i32; 2]>,
}

#[derive(Debug, Clone)]
pub(crate) struct SizePreset {
    pub(crate) label: String,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

#[derive(Debug, Clone)]
pub(crate) struct DesktopApp {
    pub(crate) name: String,
    pub(crate) exec: String,
}
