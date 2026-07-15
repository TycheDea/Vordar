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

/// Resolve `mode` + `resolution` into a winit `Fullscreen` value for `monitor`.
/// Exclusive fullscreen picks the video mode closest to the target size
/// (`Auto` resolves to the monitor's native size); a missing monitor falls
/// back to borderless.
#[cfg(feature = "winit")]
pub fn resolve_fullscreen(
    mode:       &WindowMode,
    resolution: &Resolution,
    monitor:    Option<winit::monitor::MonitorHandle>,
) -> Option<winit::window::Fullscreen> {
    match mode {
        WindowMode::Windowed   => None,
        WindowMode::Borderless => Some(winit::window::Fullscreen::Borderless(None)),
        WindowMode::Fullscreen => monitor
            .and_then(|m| {
                let (tw, th) = match resolution {
                    Resolution::Fixed(w, h) => (*w, *h),
                    Resolution::Auto => {
                        let s = m.size();
                        (s.width, s.height)
                    }
                };
                m.video_modes().min_by_key(|vm| {
                    let s  = vm.size();
                    let dw = (s.width  as i64 - tw as i64).abs();
                    let dh = (s.height as i64 - th as i64).abs();
                    dw + dh
                })
            })
            .map(winit::window::Fullscreen::Exclusive)
            .or(Some(winit::window::Fullscreen::Borderless(None))),
    }
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
