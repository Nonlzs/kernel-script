use egui::{Context, Id, LayerId, Order};

use crate::lua_runtime::DrawCommand;

pub fn paint_commands(ctx: &Context, commands: Vec<DrawCommand>) {
    if commands.is_empty() {
        return;
    }
    let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("draw_overlay")));
    for command in commands {
        match command {
            DrawCommand::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                thickness,
            } => {
                painter.line_segment(
                    [egui::pos2(x1, y1), egui::pos2(x2, y2)],
                    egui::Stroke::new(
                        thickness,
                        egui::Color32::from_rgba_unmultiplied(
                            color[0], color[1], color[2], color[3],
                        ),
                    ),
                );
            }
            DrawCommand::Rect {
                x,
                y,
                w,
                h,
                color,
                thickness,
            } => {
                painter.rect_stroke(
                    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
                    0.0,
                    egui::Stroke::new(
                        thickness,
                        egui::Color32::from_rgba_unmultiplied(
                            color[0], color[1], color[2], color[3],
                        ),
                    ),
                );
            }
            DrawCommand::FilledRect { x, y, w, h, color } => {
                painter.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)),
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
                );
            }
            DrawCommand::Circle {
                x,
                y,
                radius,
                color,
                thickness,
            } => {
                painter.circle_stroke(
                    egui::pos2(x, y),
                    radius,
                    egui::Stroke::new(
                        thickness,
                        egui::Color32::from_rgba_unmultiplied(
                            color[0], color[1], color[2], color[3],
                        ),
                    ),
                );
            }
            DrawCommand::FilledCircle {
                x,
                y,
                radius,
                color,
            } => {
                painter.circle_filled(
                    egui::pos2(x, y),
                    radius,
                    egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
                );
            }
            DrawCommand::Text {
                x,
                y,
                text,
                color,
                size,
            } => {
                let color =
                    egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
                let galley = ctx.fonts(|fonts| {
                    fonts.layout_no_wrap(text, egui::FontId::proportional(size), color)
                });
                painter.galley(egui::pos2(x, y), galley, egui::Color32::TRANSPARENT);
            }
        }
    }
}
