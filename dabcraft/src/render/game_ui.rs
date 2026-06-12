use crate::world::block::BlockId;

/// Center-screen crosshair: two hairline segments on the Foreground layer.
pub fn draw_crosshair(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("crosshair"),
    ));
    let c = ctx.content_rect().center();
    let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200));
    painter.line_segment([c - egui::vec2(8.0, 0.0), c + egui::vec2(8.0, 0.0)], stroke);
    painter.line_segment([c - egui::vec2(0.0, 8.0), c + egui::vec2(0.0, 8.0)], stroke);
}

/// Dimmed full-screen veil with a centered notice, shown while the cursor
/// is released (Escape) and gameplay is frozen.
pub fn draw_pause_overlay(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("pause"),
    ));
    let rect = ctx.content_rect();
    painter.rect_filled(rect, 0.0, egui::Color32::from_black_alpha(120));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Paused",
        egui::FontId::proportional(28.0),
        egui::Color32::WHITE,
    );
    painter.text(
        rect.center() + egui::vec2(0.0, 28.0),
        egui::Align2::CENTER_CENTER,
        "Click to resume",
        egui::FontId::proportional(16.0),
        egui::Color32::from_gray(200),
    );
}

/// Bottom-center hotbar: 9 color swatches (block colors mirror the terrain
/// palette until M6 textures), white border on the selected slot, selected
/// block name above.
pub fn draw_hotbar(ctx: &egui::Context, slots: &[BlockId; 9], selected: usize) {
    egui::Area::new(egui::Id::new("hotbar"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -12.0))
        .interactable(false)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new(slots[selected].display_name())
                        .color(egui::Color32::WHITE)
                        .strong(),
                );
                ui.horizontal(|ui| {
                    for (i, block) in slots.iter().enumerate() {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
                        let c = block.color();
                        // Rgba is linear-space; conversion to Color32 applies
                        // the sRGB transfer, matching the shader's look.
                        let fill = egui::Color32::from(egui::Rgba::from_rgb(c[0], c[1], c[2]));
                        ui.painter().rect_filled(rect.shrink(2.0), 4.0, fill);
                        let stroke = if i == selected {
                            egui::Stroke::new(2.0, egui::Color32::WHITE)
                        } else {
                            egui::Stroke::new(1.0, egui::Color32::from_gray(90))
                        };
                        ui.painter().rect_stroke(rect.shrink(2.0), 4.0, stroke, egui::StrokeKind::Outside);
                    }
                });
            });
        });
}
