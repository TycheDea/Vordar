// Action bar — MMO-style skill slots, bottom-center.
//
// ActionBarSyncSystem publishes the slots from CastState each display frame;
// the `draw` callback (a UiLayers layer) renders them. Non-interactable v1:
// the keybinds are the input (LMB = bolt, Q = blast); clicking slots is later
// polish.

use crate::CastState;
use engine_app::scheduler::System;
use engine_core::traits::Resources;
use engine_core::World;
use hecs::Entity;
use vordar_game::Player;

#[derive(Clone)]
pub struct SkillSlot {
    pub label: &'static str,
    pub keybind: &'static str,
    /// Remaining-cooldown fraction (1.0 = just fired → 0.0 = ready); None = ready.
    pub cooldown_frac: Option<f32>,
    /// Disabled slots draw dimmed (e.g. blast offline, empty slots).
    pub enabled: bool,
}

#[derive(Default)]
pub struct ActionBarState {
    pub open: bool,
    pub slots: Vec<SkillSlot>,
}

/// Fills the bar from CastState. Blast is a server-resolved mechanic, so its
/// slot is enabled only when playing online (NetClientState present).
pub struct ActionBarSyncSystem;

impl System for ActionBarSyncSystem {
    fn run(&mut self, world: &mut World, resources: &mut Resources, _delta: f32) {
        let has_player = crate::net::own_entity(resources).is_some()
            || world.query::<(Entity, &Player)>().iter().next().is_some();
        let online = resources.contains::<crate::net::NetClientState>();
        let (bolt, blast) = resources
            .get::<CastState>()
            .map(|c| (c.bolt.remaining_frac(), c.blast.remaining_frac()))
            .unwrap_or((None, None));

        let Some(bar) = resources.get_mut::<ActionBarState>() else { return };
        bar.open = has_player;
        bar.slots = vec![
            SkillSlot { label: "Bolt", keybind: "LMB", cooldown_frac: bolt, enabled: true },
            SkillSlot { label: "Blast", keybind: "Q", cooldown_frac: blast, enabled: online },
            SkillSlot { label: "", keybind: "", cooldown_frac: None, enabled: false },
            SkillSlot { label: "", keybind: "", cooldown_frac: None, enabled: false },
            SkillSlot { label: "", keybind: "", cooldown_frac: None, enabled: false },
        ];
    }
}

const SLOT_SIDE: f32 = 48.0;
const SLOT_GAP: f32 = 6.0;

/// Draw the bar. Non-interactable so it never steals game input.
pub fn draw(ctx: &egui::Context, resources: &Resources) {
    use egui::{Align2, Area, Color32, Id, Pos2, Stroke, Vec2};

    let Some(bar) = resources.get::<ActionBarState>().filter(|b| b.open) else { return };
    if bar.slots.is_empty() {
        return;
    }

    Area::new(Id::new("action_bar"))
        .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -12.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let n = bar.slots.len() as f32;
            let size = Vec2::new(n * SLOT_SIDE + (n - 1.0) * SLOT_GAP, SLOT_SIDE);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter();

            for (i, slot) in bar.slots.iter().enumerate() {
                let min = Pos2::new(
                    rect.min.x + i as f32 * (SLOT_SIDE + SLOT_GAP),
                    rect.min.y,
                );
                let r = egui::Rect::from_min_size(min, Vec2::splat(SLOT_SIDE));

                painter.rect_filled(r, 4.0, Color32::from_black_alpha(160));
                let border = if slot.enabled { Color32::from_gray(140) } else { Color32::from_gray(60) };
                painter.rect_stroke(r, 4.0, Stroke::new(1.5, border), egui::StrokeKind::Inside);

                if !slot.enabled {
                    continue;
                }

                let text = if slot.cooldown_frac.is_some() {
                    Color32::from_gray(120)
                } else {
                    Color32::WHITE
                };
                painter.text(
                    Pos2::new(r.min.x + 4.0, r.min.y + 3.0),
                    Align2::LEFT_TOP,
                    slot.keybind,
                    egui::FontId::monospace(10.0),
                    Color32::from_gray(170),
                );
                painter.text(
                    Pos2::new(r.center().x, r.max.y - 4.0),
                    Align2::CENTER_BOTTOM,
                    slot.label,
                    egui::FontId::proportional(11.0),
                    text,
                );

                // Cooldown: dark sweep over the remaining fraction (top-down,
                // like every MMO), plus a cyan readiness underline.
                if let Some(frac) = slot.cooldown_frac {
                    let frac = frac.clamp(0.0, 1.0);
                    let sweep = egui::Rect::from_min_size(
                        r.min,
                        Vec2::new(SLOT_SIDE, SLOT_SIDE * frac),
                    );
                    painter.rect_filled(sweep, 4.0, Color32::from_black_alpha(150));
                    let under = egui::Rect::from_min_size(
                        Pos2::new(r.min.x, r.max.y - 3.0),
                        Vec2::new(SLOT_SIDE * (1.0 - frac), 3.0),
                    );
                    painter.rect_filled(under, 1.0, Color32::from_rgb(80, 240, 255));
                }
            }
        });
}
