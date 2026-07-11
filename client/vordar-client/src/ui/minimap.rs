// Minimap — disc with entity dots, portal markers, heading tick (top-right).
//
// Moved out of the engine (Phase 8): the minimap is game presentation, not
// engine machinery. HudSyncSystem publishes HudState each display frame; the
// `draw` callback (registered as a UiLayers layer) renders it.

/// One blip on the minimap: world XZ + linear RGB.
#[derive(Clone)]
pub struct HudDot {
    pub pos: glam::Vec2,
    pub color: [f32; 3],
}

#[derive(Clone, Default)]
pub struct HudState {
    /// Nothing is drawn until the game flips this on.
    pub open: bool,
    /// Tracked entity (usually the player), world XZ — the disc's center.
    pub center: Option<glam::Vec2>,
    /// Camera yaw — drawn as a view-direction tick on the disc rim.
    pub heading: f32,
    pub dots: Vec<HudDot>,
    /// Diamond markers (e.g. portals) — drawn even when slightly out of range,
    /// clamped to the rim so they double as a compass.
    pub markers: Vec<glam::Vec2>,
    /// World units mapped to the disc radius.
    pub range: f32,
    /// Caption under the disc (e.g. zone name).
    pub label: String,
    /// Set while the network connection is down and a redial is being
    /// retried in the background (networking audit 2026-07-11, finding 7).
    /// None when connected, or offline (no NetClientState at all).
    pub reconnecting: Option<u32>,
}

const DISC_RADIUS: f32 = 90.0;

pub(crate) fn to_color32(c: [f32; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
    )
}

/// Draw the minimap. Non-interactable so it never steals game input.
/// North-up: world −Z is the top of the disc, world +X is right.
pub fn draw(ctx: &egui::Context, resources: &engine_core::traits::Resources) {
    use egui::{Align2, Area, Color32, Id, Pos2, Stroke, Vec2};

    // Independent of `open`: the reconnect banner must show even when there
    // is no local player yet (e.g. still relogging after a redial).
    if let Some(attempt) = resources.get::<HudState>().and_then(|h| h.reconnecting) {
        Area::new(Id::new("hud_reconnecting"))
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 12.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.colored_label(
                    Color32::from_rgb(235, 90, 90),
                    format!("Reconnecting to server… (attempt {attempt})"),
                );
            });
    }

    let Some(hud) = resources.get::<HudState>().filter(|h| h.open) else { return };
    let Some(center) = hud.center else { return };
    let range = hud.range.max(1.0);

    Area::new(Id::new("hud_minimap"))
        .anchor(Align2::RIGHT_TOP, Vec2::new(-12.0, 12.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let side = DISC_RADIUS * 2.0 + 8.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::new(side, side + 34.0), egui::Sense::hover());
            let painter = ui.painter();
            let disc_center = Pos2::new(rect.center().x, rect.top() + 4.0 + DISC_RADIUS);

            // World XZ → disc px (north-up: +X right, +Z down/south).
            let project = |world: glam::Vec2| -> (Pos2, f32) {
                let rel = (world - center) / range * DISC_RADIUS;
                (Pos2::new(disc_center.x + rel.x, disc_center.y + rel.y), rel.length())
            };

            painter.circle_filled(disc_center, DISC_RADIUS, Color32::from_black_alpha(160));
            painter.circle_stroke(disc_center, DISC_RADIUS, Stroke::new(1.5, Color32::from_gray(120)));

            // Range midline ring — quiet distance cue.
            painter.circle_stroke(disc_center, DISC_RADIUS * 0.5, Stroke::new(0.5, Color32::from_gray(60)));

            for dot in &hud.dots {
                let (px, dist) = project(dot.pos);
                if dist > DISC_RADIUS - 2.0 {
                    continue;
                }
                painter.circle_filled(px, 3.0, to_color32(dot.color));
            }

            // Markers clamp to the rim — a portal beyond range still shows a bearing.
            for &marker in &hud.markers {
                let (mut px, dist) = project(marker);
                if dist > DISC_RADIUS - 6.0 {
                    let dir = (px - disc_center) / dist;
                    px = disc_center + dir * (DISC_RADIUS - 6.0);
                }
                let r = 5.0;
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        Pos2::new(px.x, px.y - r),
                        Pos2::new(px.x + r, px.y),
                        Pos2::new(px.x, px.y + r),
                        Pos2::new(px.x - r, px.y),
                    ],
                    Color32::from_rgb(80, 240, 255),
                    Stroke::new(1.0, Color32::WHITE),
                ));
            }

            // Self + camera facing (camera forward on the ground is
            // (−cos yaw, −sin yaw) in world XZ — same math as movement axes).
            painter.circle_filled(disc_center, 4.0, Color32::WHITE);
            let facing = Vec2::new(-hud.heading.cos(), -hud.heading.sin()) * 14.0;
            painter.line_segment(
                [disc_center, disc_center + facing],
                Stroke::new(2.0, Color32::from_gray(220)),
            );

            // North tick.
            painter.line_segment(
                [
                    Pos2::new(disc_center.x, disc_center.y - DISC_RADIUS),
                    Pos2::new(disc_center.x, disc_center.y - DISC_RADIUS + 6.0),
                ],
                Stroke::new(2.0, Color32::from_gray(200)),
            );

            // Zone label.
            let label_pos = Pos2::new(disc_center.x, rect.top() + 4.0 + DISC_RADIUS * 2.0 + 6.0);
            painter.text(
                label_pos,
                Align2::CENTER_TOP,
                &hud.label,
                egui::FontId::monospace(13.0),
                Color32::WHITE,
            );
        });
}
