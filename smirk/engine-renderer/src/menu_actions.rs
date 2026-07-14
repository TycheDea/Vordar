//! Applies menu actions deferred from the egui frame — window mode/resolution/
//! vsync, menu navigation, quit.

use crate::menu::{MenuAction, MenuScreen, MenuState, SettingsDraft};
use crate::state::RendererState;
use engine_app::config::{Resolution, WindowConfig, WindowMode};
use engine_core::traits::Resources;
use std::sync::Arc;
use winit::window::Window;

pub(crate) fn apply_menu_actions(actions: Vec<MenuAction>, resources: &mut Resources) {
    // Collect UpdateDraft actions — only keep the last one (multiple can arrive per frame).
    let mut latest_draft: Option<SettingsDraft> = None;
    let mut other_actions: Vec<MenuAction> = Vec::new();
    for action in actions {
        match action {
            MenuAction::UpdateDraft(d) => { latest_draft = Some(d); }
            other => other_actions.push(other),
        }
    }

    // Apply latest draft (from egui widget mutations)
    if let Some(d) = latest_draft {
        if let Some(m) = resources.get_mut::<MenuState>() {
            m.draft = d;
        }
    }

    for action in other_actions {
        match action {
            MenuAction::Resume => {
                if let Some(m) = resources.get_mut::<MenuState>() { m.open = false; }
            }
            MenuAction::OpenSettings => {
                let draft = resources.get::<WindowConfig>().map(SettingsDraft::from_config);
                if let Some(m) = resources.get_mut::<MenuState>() {
                    if let Some(d) = draft { m.draft = d; }
                    m.screen   = MenuScreen::Settings;
                    m.selected = 0;
                }
            }
            MenuAction::SaveAndBack => {
                let draft = resources.get::<MenuState>().map(|m| m.draft.clone());
                if let Some(d) = draft {
                    let new_cfg = d.into_config();
                    if let Some(w) = resources.get::<Arc<Window>>() {
                        w.set_title(&new_cfg.title);
                        // Only request size change when windowed — fullscreen modes ignore it.
                        if matches!(new_cfg.mode, WindowMode::Windowed) {
                            if let Resolution::Fixed(ww, wh) = new_cfg.resolution {
                                let _ = w.request_inner_size(
                                    winit::dpi::PhysicalSize::new(ww, wh),
                                );
                            }
                        }
                        let fullscreen = match new_cfg.mode {
                            WindowMode::Windowed   => None,
                            WindowMode::Borderless =>
                                Some(winit::window::Fullscreen::Borderless(None)),
                            WindowMode::Fullscreen =>
                                w.current_monitor()
                                 .and_then(|m| m.video_modes().next())
                                 .map(winit::window::Fullscreen::Exclusive)
                                 .or(Some(winit::window::Fullscreen::Borderless(None))),
                        };
                        w.set_fullscreen(fullscreen);
                    }
                    // Apply vsync change to the wgpu surface
                    if let Some(state) = resources.get_mut::<RendererState>() {
                        state.config.present_mode = if new_cfg.vsync {
                            wgpu::PresentMode::AutoVsync
                        } else {
                            wgpu::PresentMode::AutoNoVsync
                        };
                        state.surface.configure(&state.device, &state.config);
                    }
                    resources.insert(new_cfg);
                }
                if let Some(m) = resources.get_mut::<MenuState>() {
                    m.screen   = MenuScreen::Main;
                    m.selected = 1;
                }
            }
            MenuAction::CancelAndBack => {
                // Discard draft — reset from current config
                let draft = resources.get::<WindowConfig>().map(SettingsDraft::from_config);
                if let Some(m) = resources.get_mut::<MenuState>() {
                    if let Some(d) = draft { m.draft = d; }
                    m.screen   = MenuScreen::Main;
                    m.selected = 1;
                }
            }
            MenuAction::Quit => std::process::exit(0),
            MenuAction::HoverSelect(i) => {
                if let Some(m) = resources.get_mut::<MenuState>() {
                    m.selected = i;
                }
            }
            MenuAction::UpdateDraft(_) => unreachable!(), // handled above
        }
    }
}
