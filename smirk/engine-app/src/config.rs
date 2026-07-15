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
use std::path::Path;

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

/// Serialize a WindowConfig to RON format and write it to disk with a format header.
/// Returns Ok(()) on success; returns Err with a message on any serialization or write error.
pub fn save_window_config(path: &Path, config: &WindowConfig) -> Result<(), String> {
    match ron::ser::to_string_pretty(&config, ron::ser::PrettyConfig::default()) {
        Ok(s) => {
            let header = "// Engine window configuration.\n\
                          // Loaded by App::configure(\"content/config/engine.ron\").\n\
                          // resolution: Auto | Fixed(1280, 720)\n\
                          // mode:       Windowed | Borderless | Fullscreen\n\
                          // vsync:      true | false\n\
                          // max_fps:    None = monitor refresh rate, Some(60) = explicit cap\n";
            std::fs::write(path, format!("{header}{s}\n"))
                .map_err(|e| format!("failed to write config: {e}"))
        }
        Err(e) => Err(format!("failed to serialize config: {e}")),
    }
}

/// Load a WindowConfig from a RON file on disk.
/// Returns Some(config) on success; returns None if the file cannot be read or parsed.
pub fn reload_config(path: &Path) -> Option<WindowConfig> {
    let s = std::fs::read_to_string(path).ok()?;
    ron::from_str::<WindowConfig>(&s).ok()
}
