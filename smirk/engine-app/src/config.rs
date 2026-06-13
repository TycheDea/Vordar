// Window and engine configuration — deserialized from a RON file via App::configure().
//
// Example assets/config/engine.ron:
//   (
//       title: "My Game",
//       resolution: Auto,
//       mode: Borderless,
//       vsync: true,
//       max_fps: None,   // None = cap to monitor refresh rate; Some(n) = explicit cap
//   )

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowConfig {
    pub title:      String,
    pub resolution: Resolution,
    pub mode:       WindowMode,
    /// Enable vsync (AutoVsync) or disable (AutoNoVsync).
    #[serde(default = "default_true")]
    pub vsync:      bool,
    /// Cap frame rate. `None` = cap to monitor refresh rate; `Some(n)` = explicit cap.
    #[serde(default)]
    pub max_fps:    Option<u32>,
}

fn default_true() -> bool { true }

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title:      "Smirk".to_string(),
            resolution: Resolution::Auto,
            mode:       WindowMode::Borderless,
            vsync:      true,
            max_fps:    None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Resolution {
    /// Explicit pixel dimensions.
    Fixed(u32, u32),
    /// Use the primary monitor's native resolution.
    Auto,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum WindowMode {
    /// Normal bordered window at the configured size.
    Windowed,
    /// Borderless fullscreen on the current monitor (no resolution change, zero latency).
    Borderless,
    /// Exclusive fullscreen — takes full ownership of the display.
    Fullscreen,
}
