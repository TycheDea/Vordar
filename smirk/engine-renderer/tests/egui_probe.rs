// Regression guard: game input must reach the game when the pointer is over
// open playfield, even with the HUD overlays drawn.
//
// RenderSystem runs the egui frame with begin_pass/end_pass. It must NOT use
// `Context::run_ui`: run_ui wraps the frame in a full-screen background Ui
// and allocates it as a central panel, making egui claim the entire viewport.
// egui-winit then reports clicks and wheel events as consumed whenever no
// button is held (`egui_wants_pointer_input` = pointer-over-egui && !any_down),
// which broke left-click casting (unless right-click was already held) and
// mouse-wheel zoom entirely.
//
// This test mirrors the production frame (non-interactable HUD area, pointer
// mid-screen) and asserts what egui-winit consults to decide `consumed`.

fn hud_like(ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("hud_minimap"))
        .anchor(egui::Align2::RIGHT_TOP, egui::Vec2::new(-12.0, 12.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::Vec2::new(188.0, 222.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(rect.center(), 90.0, egui::Color32::from_black_alpha(160));
        });
}

fn run_frame(ctx: &egui::Context, input: egui::RawInput) {
    ctx.begin_pass(input);
    hud_like(ctx);
    let _ = ctx.end_pass();
}

#[test]
fn pointer_mid_screen_is_not_claimed_by_egui() {
    let ctx = egui::Context::default();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 720.0));

    // Frame 1: establish layers.
    run_frame(&ctx, egui::RawInput { screen_rect: Some(screen), ..Default::default() });

    // Frame 2: pointer sits mid-screen (no buttons down).
    let mut input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
    input.events.push(egui::Event::PointerMoved(egui::pos2(640.0, 360.0)));
    run_frame(&ctx, input);

    // This is what egui-winit consults to decide `consumed` for
    // MouseInput / MouseWheel events.
    assert!(
        !ctx.egui_wants_pointer_input(),
        "egui claims the pointer mid-screen: is_pointer_over_egui={} is_using_pointer={}",
        ctx.is_pointer_over_egui(),
        ctx.egui_is_using_pointer()
    );
}
