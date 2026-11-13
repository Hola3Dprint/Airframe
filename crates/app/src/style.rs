use eframe::egui::*;

#[derive(Clone, Copy)]
pub struct Palette {
    pub canvas: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub edge: Color32,
    pub grid: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub good: Color32,
    pub warning: Color32,
}
pub fn palette(light: bool) -> Palette {
    let rgb = Color32::from_rgb;
    if light {
        Palette {
            canvas: rgb(236, 241, 248),
            surface: rgb(248, 250, 253),
            raised: rgb(229, 236, 247),
            edge: rgb(188, 201, 219),
            grid: rgb(190, 205, 225),
            text: rgb(25, 38, 59),
            muted: rgb(78, 99, 126),
            accent: rgb(31, 98, 187),
            good: rgb(11, 111, 76),
            warning: rgb(162, 78, 20),
        }
    } else {
        Palette {
            canvas: rgb(13, 19, 30),
            surface: rgb(19, 27, 41),
            raised: rgb(28, 40, 59),
            edge: rgb(43, 58, 80),
            grid: rgb(42, 57, 78),
            text: rgb(225, 234, 246),
            muted: rgb(150, 170, 195),
            accent: rgb(130, 181, 255),
            good: rgb(104, 223, 171),
            warning: rgb(255, 174, 112),
        }
    }
}
pub fn install(ctx: &Context, light: bool) {
    let p = palette(light);
    let mut s = Style {
        visuals: if light {
            Visuals::light()
        } else {
            Visuals::dark()
        },
        ..Style::default()
    };
    s.spacing.item_spacing = vec2(8.0, 8.0);
    s.spacing.button_padding = vec2(10.0, 5.0);
    s.spacing.interact_size.y = 26.0;
    s.visuals.panel_fill = p.surface;
    s.visuals.window_fill = p.surface;
    s.visuals.extreme_bg_color = p.canvas;
    s.visuals.faint_bg_color = p.raised;
    s.visuals.window_stroke = Stroke::new(1.0_f32, p.edge);
    s.visuals.override_text_color = Some(p.text);
    s.visuals.selection.bg_fill = p.raised;
    s.visuals.selection.stroke = Stroke::new(1.0_f32, p.accent);
    for (w, bg, fg) in [
        (&mut s.visuals.widgets.inactive, p.raised, p.edge),
        (&mut s.visuals.widgets.hovered, p.raised, p.accent),
        (&mut s.visuals.widgets.active, p.raised, p.accent),
        (&mut s.visuals.widgets.open, p.raised, p.accent),
        (&mut s.visuals.widgets.noninteractive, p.surface, p.edge),
    ] {
        w.bg_fill = bg;
        w.weak_bg_fill = bg;
        w.bg_stroke = Stroke::new(1.0_f32, fg);
        w.fg_stroke = Stroke::new(1.0_f32, p.text);
        w.corner_radius = CornerRadius::same(6);
    }
    ctx.set_style(s);
}
pub fn eyebrow(ui: &mut Ui, text: &str, p: Palette) {
    ui.label(
        RichText::new(text)
            .monospace()
            .size(10.5)
            .color(p.muted)
            .strong(),
    );
}
pub fn tag(text: &str, color: Color32) -> RichText {
    RichText::new(text)
        .monospace()
        .size(11.0)
        .color(color)
        .strong()
}
pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(19.0).strong());
}
pub fn panel(p: Palette, margin: i8) -> Frame {
    Frame::new()
        .fill(p.surface)
        .inner_margin(Margin::same(margin))
}
