// Pause menu — Escape to open, WASD/arrows to navigate, Enter or click to activate.
//
// MenuState is stored in Resources. MenuSystem handles keyboard navigation.
// Egui rendering is performed inside RenderSystem (same encoder pass as the 3D scene).

use engine_app::config::{Resolution, WindowConfig, WindowMode};
use engine_app::input::KeyboardState;
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;
use winit::keyboard::KeyCode;

// ── Public state ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum MenuScreen {
    Main,
    Settings,
}

/// Live-edited copy of WindowConfig while the Settings screen is open.
/// Sent back to MenuState every frame via UpdateDraft so changes persist
/// across multiple egui frames.
#[derive(Clone, Debug)]
pub struct SettingsDraft {
    pub res_fixed_w: u32,
    pub res_fixed_h: u32,
    pub res_auto:    bool,
    pub mode:        WindowMode,
    pub vsync:       bool,
    pub max_fps_val: u32, // 0 = use monitor refresh rate (None in config)
    pub title:       String,
}

impl SettingsDraft {
    pub fn from_config(cfg: &WindowConfig) -> Self {
        let (w, h, auto) = match cfg.resolution {
            Resolution::Fixed(w, h) => (w, h, false),
            Resolution::Auto        => (1920, 1080, true),
        };
        Self {
            res_fixed_w: w,
            res_fixed_h: h,
            res_auto:    auto,
            mode:        cfg.mode.clone(),
            vsync:       cfg.vsync,
            max_fps_val: cfg.max_fps.unwrap_or(0),
            title:       cfg.title.clone(),
        }
    }

    pub fn into_config(self) -> WindowConfig {
        WindowConfig {
            title:      self.title,
            resolution: if self.res_auto {
                Resolution::Auto
            } else {
                Resolution::Fixed(self.res_fixed_w, self.res_fixed_h)
            },
            mode:    self.mode,
            vsync:   self.vsync,
            max_fps: if self.max_fps_val == 0 { None } else { Some(self.max_fps_val) },
        }
    }
}

#[derive(Clone)]
pub struct MenuState {
    pub open:     bool,
    pub screen:   MenuScreen,
    pub selected: usize,
    was_escape:   bool,
    was_up:       bool,
    was_down:     bool,
    was_enter:    bool,
    pub draft:    SettingsDraft,
}

impl MenuState {
    pub fn new(cfg: &WindowConfig) -> Self {
        Self {
            open:       false,
            screen:     MenuScreen::Main,
            selected:   0,
            was_escape: false,
            was_up:     false,
            was_down:   false,
            was_enter:  false,
            draft:      SettingsDraft::from_config(cfg),
        }
    }
}

impl Default for MenuState {
    fn default() -> Self { Self::new(&WindowConfig::default()) }
}

// ── Actions ───────────────────────────────────────────────────────────────────

pub(crate) enum MenuAction {
    Resume,
    OpenSettings,
    SaveAndBack,           // apply draft → config + window, return to Main
    CancelAndBack,         // discard draft, return to Main
    Quit,
    HoverSelect(usize),
    UpdateDraft(SettingsDraft), // egui widget changes propagated back each frame
}

// ── egui helpers ──────────────────────────────────────────────────────────────

/// Clickable menu item: no frame, no hover square — color change only.
fn menu_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (color, size) = if selected {
        (egui::Color32::WHITE, 22.0_f32)
    } else {
        (egui::Color32::from_rgb(140, 140, 140), 20.0_f32)
    };
    let text = egui::RichText::new(label).color(color).size(size);
    ui.add(egui::Label::new(text).sense(egui::Sense::click()))
}

// ── egui draw ────────────────────────────────────────────────────────────────

/// Draw the full menu overlay. Writes any user actions into `out`.
pub(crate) fn draw_menu(
    ctx:         &egui::Context,
    menu:        &MenuState,
    monitor_fps: Option<u32>,
    out:         &mut Vec<MenuAction>,
) {
    use egui::{Align2, Area, Color32, Id, Vec2};

    // Full-screen semi-transparent overlay
    Area::new(Id::new("menu_overlay"))
        .fixed_pos(egui::Pos2::ZERO)
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            ui.painter().rect_filled(
                ctx.viewport_rect(),
                0.0,
                Color32::from_black_alpha(190),
            );
        });

    // Centered panel — no frame, no background, just text
    Area::new(Id::new("menu_panel"))
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.set_width(340.0);
            match menu.screen {
                MenuScreen::Main     => draw_main(ui, menu, out),
                MenuScreen::Settings => draw_settings(ui, menu, monitor_fps, out),
            }
        });
}

fn draw_main(ui: &mut egui::Ui, menu: &MenuState, out: &mut Vec<MenuAction>) {
    use egui::{Color32, RichText};

    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("PAUSED").color(Color32::WHITE).size(34.0).strong());
    });
    ui.add_space(32.0);

    for (i, label) in ["Resume", "Settings", "Quit"].iter().enumerate() {
        ui.vertical_centered(|ui| {
            let r = menu_item(ui, label, menu.selected == i);
            if r.hovered() { out.push(MenuAction::HoverSelect(i)); }
            if r.clicked() {
                out.push(match i {
                    0 => MenuAction::Resume,
                    1 => MenuAction::OpenSettings,
                    _ => MenuAction::Quit,
                });
            }
        });
        ui.add_space(10.0);
    }
    ui.add_space(16.0);
}

fn draw_settings(
    ui:          &mut egui::Ui,
    menu:        &MenuState,
    monitor_fps: Option<u32>,
    out:         &mut Vec<MenuAction>,
) {
    use egui::{Color32, ComboBox, DragValue, Id, RichText};

    let lc = Color32::from_rgb(130, 130, 130); // label / secondary color
    let wc = Color32::WHITE;

    ui.add_space(20.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("SETTINGS").color(wc).size(28.0).strong());
    });
    ui.add_space(24.0);

    let mut draft = menu.draft.clone();

    // Window Mode
    ui.label(RichText::new("Window Mode").color(lc).size(14.0));
    ui.add_space(4.0);
    ComboBox::from_id_salt(Id::new("mode_combo"))
        .width(280.0)
        .selected_text(RichText::new(match &draft.mode {
            WindowMode::Windowed   => "Windowed",
            WindowMode::Borderless => "Borderless Fullscreen",
            WindowMode::Fullscreen => "Exclusive Fullscreen",
        }).color(wc).size(15.0))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut draft.mode, WindowMode::Windowed,   "Windowed");
            ui.selectable_value(&mut draft.mode, WindowMode::Borderless, "Borderless Fullscreen");
            ui.selectable_value(&mut draft.mode, WindowMode::Fullscreen, "Exclusive Fullscreen");
        });
    ui.add_space(14.0);

    // Resolution
    ui.label(RichText::new("Resolution").color(lc).size(14.0));
    ui.add_space(4.0);
    ui.checkbox(&mut draft.res_auto, RichText::new("Auto  (native monitor resolution)").color(wc).size(15.0));
    if !draft.res_auto {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add(DragValue::new(&mut draft.res_fixed_w).range(320..=7680).speed(4.0));
            ui.label(RichText::new("×").color(lc).size(15.0));
            ui.add(DragValue::new(&mut draft.res_fixed_h).range(240..=4320).speed(4.0));
        });
    }
    ui.add_space(14.0);

    // VSync
    ui.checkbox(&mut draft.vsync, RichText::new("VSync").color(wc).size(15.0));
    ui.add_space(14.0);

    // Max FPS — disabled and shows monitor fps when vsync is on
    ui.label(RichText::new("Max FPS").color(lc).size(14.0));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if draft.vsync {
            // Show the monitor refresh rate as a read-only hint; the field is locked
            let display_fps = monitor_fps.unwrap_or(draft.max_fps_val);
            let mut display = display_fps;
            ui.add_enabled(false, DragValue::new(&mut display).range(0..=1000));
            ui.label(RichText::new("  capped by VSync").color(lc).size(13.0));
        } else {
            ui.add(DragValue::new(&mut draft.max_fps_val).range(0..=1000).speed(1.0));
            ui.label(RichText::new("  (0 = monitor refresh rate)").color(lc).size(13.0));
        }
    });
    ui.add_space(28.0);

    // Propagate draft changes back every frame
    out.push(MenuAction::UpdateDraft(draft));

    // Save / Back
    ui.vertical_centered(|ui| {
        let r = menu_item(ui, "Save", menu.selected == 0);
        if r.hovered() { out.push(MenuAction::HoverSelect(0)); }
        if r.clicked() { out.push(MenuAction::SaveAndBack); }
    });
    ui.add_space(10.0);
    ui.vertical_centered(|ui| {
        let r = menu_item(ui, "Back", menu.selected == 1);
        if r.hovered() { out.push(MenuAction::HoverSelect(1)); }
        if r.clicked() { out.push(MenuAction::CancelAndBack); }
    });
    ui.add_space(16.0);
}

// ── MenuSystem ────────────────────────────────────────────────────────────────

/// Handles keyboard navigation (Escape, WASD/arrows, Enter).
pub struct MenuSystem;

impl System for MenuSystem {
    fn run(&mut self, _world: &mut World, resources: &mut Resources, _delta: f32) {
        let kb = match resources.get::<KeyboardState>() {
            Some(k) => k,
            None => return,
        };
        let escape = kb.is_pressed(KeyCode::Escape);
        let up     = kb.is_pressed(KeyCode::KeyW) || kb.is_pressed(KeyCode::ArrowUp);
        let down   = kb.is_pressed(KeyCode::KeyS) || kb.is_pressed(KeyCode::ArrowDown);
        let enter  = kb.is_pressed(KeyCode::Enter) || kb.is_pressed(KeyCode::NumpadEnter);

        let menu = match resources.get_mut::<MenuState>() {
            Some(m) => m,
            None    => return,
        };

        // Escape: toggle open / back to main / close
        if escape && !menu.was_escape {
            if menu.open {
                match menu.screen {
                    MenuScreen::Settings => {
                        menu.screen   = MenuScreen::Main;
                        menu.selected = 0;
                    }
                    MenuScreen::Main => { menu.open = false; }
                }
            } else {
                menu.open     = true;
                menu.screen   = MenuScreen::Main;
                menu.selected = 0;
            }
        }
        menu.was_escape = escape;

        if !menu.open { return; }

        let item_count = match menu.screen {
            MenuScreen::Main     => 3, // Resume, Settings, Quit
            MenuScreen::Settings => 2, // Save, Back
        };

        if up && !menu.was_up && menu.selected > 0 {
            menu.selected -= 1;
        }
        if down && !menu.was_down {
            menu.selected = (menu.selected + 1).min(item_count - 1);
        }
        menu.was_up   = up;
        menu.was_down = down;

        if enter && !menu.was_enter {
            match (&menu.screen, menu.selected) {
                (MenuScreen::Main, 0) => { menu.open = false; }                                // Resume
                (MenuScreen::Main, 1) => {
                    // Refresh draft from current config before entering settings
                    // (handled in apply_menu_actions for OpenSettings action)
                    menu.screen   = MenuScreen::Settings;
                    menu.selected = 0;
                }
                (MenuScreen::Main, _) => std::process::exit(0),                               // Quit
                (MenuScreen::Settings, 0) => { /* SaveAndBack handled via action */ }
                (MenuScreen::Settings, _) => {                                                  // Back
                    menu.screen   = MenuScreen::Main;
                    menu.selected = 1;
                }
            }
        }
        menu.was_enter = enter;
    }
}
