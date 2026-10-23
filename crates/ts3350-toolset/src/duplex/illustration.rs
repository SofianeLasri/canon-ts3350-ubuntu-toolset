//! Diagram of the stack flip, drawn with Cairo so it follows the light/dark theme.

use std::f64::consts::PI;

use adw::prelude::*;
use gtk::cairo;
use ts3350_core::i18n::i18n;

const ORANGE: (f64, f64, f64) = (0.91, 0.33, 0.13);

type Painter = fn(&cairo::Context, (f64, f64, f64), f64, f64);

pub fn widget() -> gtk::Box {
    let row = gtk::Box::builder().spacing(12).halign(gtk::Align::Center).homogeneous(true).build();
    let steps: [(&str, Painter); 3] = [
        (i18n("Take the stack out"), draw_take),
        (i18n("Flip it over"), draw_flip),
        (i18n("Top of the pages\nfirst"), draw_load),
    ];
    for (caption, painter) in steps {
        let area = gtk::DrawingArea::builder().content_width(140).content_height(120).build();
        area.set_draw_func(move |area, cr, width, height| {
            let fg = area.color();
            painter(cr, (fg.red() as f64, fg.green() as f64, fg.blue() as f64), width as f64, height as f64);
        });
        let label = gtk::Label::builder().label(caption).justify(gtk::Justification::Center).build();
        label.add_css_class("caption");
        let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        column.append(&area);
        column.append(&label);
        row.append(&column);
    }
    row
}

fn set(cr: &cairo::Context, (r, g, b): (f64, f64, f64), alpha: f64) {
    cr.set_source_rgba(r, g, b, alpha);
}

/// Sheet seen from the front; `printed` adds text lines and a “top” arrow.
fn sheet(cr: &cairo::Context, fg: (f64, f64, f64), x: f64, y: f64, w: f64, h: f64, printed: bool) {
    cr.rectangle(x, y, w, h);
    set(cr, fg, 0.08);
    let _ = cr.fill_preserve();
    set(cr, fg, 0.7);
    cr.set_line_width(1.5);
    let _ = cr.stroke();
    if printed {
        set(cr, fg, 0.45);
        cr.set_line_width(2.0);
        for i in 0..4 {
            let ly = y + h * 0.42 + i as f64 * h * 0.12;
            cr.move_to(x + w * 0.18, ly);
            cr.line_to(x + w * if i == 3 { 0.55 } else { 0.82 }, ly);
        }
        let _ = cr.stroke();
        arrow_up(cr, x + w / 2.0, y + h * 0.08, h * 0.22);
    }
}

/// Small orange arrow pointing to the top of the page.
fn arrow_up(cr: &cairo::Context, cx: f64, top: f64, size: f64) {
    set(cr, ORANGE, 1.0);
    cr.move_to(cx, top);
    cr.line_to(cx - size * 0.45, top + size * 0.55);
    cr.line_to(cx + size * 0.45, top + size * 0.55);
    cr.close_path();
    let _ = cr.fill();
    cr.rectangle(cx - size * 0.15, top + size * 0.5, size * 0.3, size * 0.5);
    let _ = cr.fill();
}

fn draw_take(cr: &cairo::Context, fg: (f64, f64, f64), w: f64, h: f64) {
    let (sw, sh) = (w * 0.42, h * 0.68);
    for i in (0..3).rev() {
        let offset = i as f64 * 6.0;
        sheet(cr, fg, (w - sw) / 2.0 + offset, (h - sh) / 2.0 + 8.0 - offset, sw, sh, i == 0);
    }
}

fn draw_flip(cr: &cairo::Context, fg: (f64, f64, f64), w: f64, h: f64) {
    let (sw, sh) = (w * 0.3, h * 0.6);
    let y = (h - sh) / 2.0 + 8.0;
    sheet(cr, fg, w * 0.08, y, sw, sh, true);
    sheet(cr, fg, w * 0.62, y, sw, sh, false);
    // Arrowed arc from the printed sheet to the blank one.
    set(cr, ORANGE, 1.0);
    cr.set_line_width(2.5);
    let (cx, cy, radius) = (w / 2.0, y + 4.0, w * 0.2);
    cr.arc(cx, cy, radius, PI * 1.15, PI * 1.85);
    let _ = cr.stroke();
    let (ex, ey) = (cx + radius * (PI * 1.85).cos(), cy + radius * (PI * 1.85).sin());
    cr.move_to(ex + 6.0, ey + 2.0);
    cr.line_to(ex - 4.0, ey - 5.0);
    cr.line_to(ex - 3.0, ey + 6.0);
    cr.close_path();
    let _ = cr.fill();
}

fn draw_load(cr: &cairo::Context, fg: (f64, f64, f64), w: f64, h: f64) {
    // Printer seen from the side: body, slanted rear tray, output on the left (front).
    let (bx, by, bw, bh) = (w * 0.12, h * 0.55, w * 0.62, h * 0.3);
    rounded_rect(cr, bx, by, bw, bh, 6.0);
    set(cr, fg, 0.12);
    let _ = cr.fill_preserve();
    set(cr, fg, 0.7);
    cr.set_line_width(1.5);
    let _ = cr.stroke();

    // Rear tray.
    let base = (bx + bw - 10.0, by + 4.0);
    let top = (bx + bw + 18.0, h * 0.08);
    cr.move_to(base.0, base.1);
    cr.line_to(top.0, top.1);
    set(cr, fg, 0.7);
    cr.set_line_width(3.0);
    let _ = cr.stroke();

    // Blank stack slid into the tray, printed side facing the back.
    let _ = cr.save();
    cr.translate(base.0 - 14.0, base.1 - 6.0);
    cr.rotate((top.0 - base.0).atan2(base.1 - top.1));
    cr.rectangle(-4.0, -h * 0.4, 8.0, h * 0.38);
    set(cr, fg, 0.08);
    let _ = cr.fill_preserve();
    set(cr, fg, 0.7);
    cr.set_line_width(1.5);
    let _ = cr.stroke();
    // Arrow: the stack goes down into the tray, top of the pages first.
    set(cr, ORANGE, 1.0);
    cr.set_line_width(2.5);
    cr.move_to(-16.0, -h * 0.38);
    cr.line_to(-16.0, -h * 0.1);
    let _ = cr.stroke();
    cr.move_to(-16.0, -h * 0.02);
    cr.line_to(-22.0, -h * 0.12);
    cr.line_to(-10.0, -h * 0.12);
    cr.close_path();
    let _ = cr.fill();
    let _ = cr.restore();
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}
