//! Platen preview: low-resolution image and scan area drawn with the mouse.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::cairo;
use ts3350_core::escl::Region;
use ts3350_core::i18n::i18n;

type SelectCallback = Box<dyn Fn(Region)>;

/// A selection smaller than ~4 mm is treated as a simple click.
const MIN_SELECTION: f64 = 50.0;

pub struct Preview {
    pub root: gtk::Overlay,
    area: gtk::DrawingArea,
    placeholder: gtk::Label,
    /// Platen dimensions in 1/300 inch.
    platen: Cell<(u32, u32)>,
    image: RefCell<Option<cairo::ImageSurface>>,
    selection: Cell<Option<Region>>,
    drag_origin: Cell<Option<(f64, f64)>>,
    before_drag: Cell<Option<Region>>,
    on_select: RefCell<Option<SelectCallback>>,
}

/// Placement of the platen in the widget: offset and scale (pixels per unit).
#[derive(Clone, Copy)]
struct Layout {
    x: f64,
    y: f64,
    scale: f64,
}

impl Preview {
    pub fn new() -> Rc<Preview> {
        let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
        area.set_cursor_from_name(Some("crosshair"));
        let placeholder = gtk::Label::builder()
            .label(i18n("Click “Preview” to see what is on the glass,\nthen draw the area to scan."))
            .justify(gtk::Justification::Center)
            .wrap(true)
            .can_target(false)
            .build();
        placeholder.add_css_class("dim-label");
        let root = gtk::Overlay::builder().child(&area).vexpand(true).build();
        root.add_overlay(&placeholder);

        let preview = Rc::new(Preview {
            root,
            area,
            placeholder,
            platen: Cell::new((2550, 3508)),
            image: RefCell::new(None),
            selection: Cell::new(None),
            drag_origin: Cell::new(None),
            before_drag: Cell::new(None),
            on_select: RefCell::new(None),
        });

        let weak = Rc::downgrade(&preview);
        preview.area.set_draw_func(move |area, cr, width, height| {
            if let Some(preview) = weak.upgrade() {
                preview.draw(area, cr, width as f64, height as f64);
            }
        });

        let drag = gtk::GestureDrag::new();
        let weak = Rc::downgrade(&preview);
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(p) = weak.upgrade() {
                p.before_drag.set(p.selection.get());
                p.drag_origin.set(Some(p.to_units(x, y)));
            }
        });
        let weak = Rc::downgrade(&preview);
        drag.connect_drag_update(move |gesture, dx, dy| {
            if let (Some(p), Some((sx, sy))) = (weak.upgrade(), gesture.start_point()) {
                let start = p.drag_origin.get().unwrap_or_else(|| p.to_units(sx, sy));
                let end = p.to_units(sx + dx, sy + dy);
                p.selection.set(Some(rect(start, end)));
                p.area.queue_draw();
            }
        });
        let weak = Rc::downgrade(&preview);
        drag.connect_drag_end(move |_, _, _| {
            let Some(p) = weak.upgrade() else { return };
            p.drag_origin.set(None);
            match p.selection.get() {
                Some(r) if r.width as f64 >= MIN_SELECTION && r.height as f64 >= MIN_SELECTION => {
                    if let Some(callback) = p.on_select.borrow().as_ref() {
                        callback(r);
                    }
                }
                _ => {
                    p.selection.set(p.before_drag.get());
                    p.area.queue_draw();
                }
            }
        });
        preview.area.add_controller(drag);
        preview
    }

    pub fn connect_select(&self, callback: impl Fn(Region) + 'static) {
        *self.on_select.borrow_mut() = Some(Box::new(callback));
    }

    pub fn set_platen(&self, width: u32, height: u32) {
        self.platen.set((width.max(1), height.max(1)));
        self.area.queue_draw();
    }

    pub fn set_selection(&self, region: Option<Region>) {
        self.selection.set(region);
        self.area.queue_draw();
    }

    /// Shows the preview (image of the whole platen).
    pub fn set_image(&self, image: &image::RgbaImage) {
        *self.image.borrow_mut() = surface_from_rgba(image);
        self.placeholder.set_visible(false);
        self.area.queue_draw();
    }

    fn layout(&self, width: f64, height: f64) -> Layout {
        let (pw, ph) = self.platen.get();
        let margin = 16.0;
        let scale = ((width - 2.0 * margin) / pw as f64).min((height - 2.0 * margin) / ph as f64).max(0.01);
        Layout { x: (width - pw as f64 * scale) / 2.0, y: (height - ph as f64 * scale) / 2.0, scale }
    }

    fn to_units(&self, x: f64, y: f64) -> (f64, f64) {
        let l = self.layout(self.area.width() as f64, self.area.height() as f64);
        let (pw, ph) = self.platen.get();
        (((x - l.x) / l.scale).clamp(0.0, pw as f64), ((y - l.y) / l.scale).clamp(0.0, ph as f64))
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &cairo::Context, width: f64, height: f64) {
        let l = self.layout(width, height);
        let (pw, ph) = self.platen.get();
        let (w, h) = (pw as f64 * l.scale, ph as f64 * l.scale);
        let fg = area.color();
        let (r, g, b) = (fg.red() as f64, fg.green() as f64, fg.blue() as f64);

        // Platen background.
        cr.rectangle(l.x, l.y, w, h);
        cr.set_source_rgba(r, g, b, 0.06);
        let _ = cr.fill();

        if let Some(surface) = self.image.borrow().as_ref() {
            let _ = cr.save();
            cr.translate(l.x, l.y);
            cr.scale(w / surface.width() as f64, h / surface.height() as f64);
            let _ = cr.set_source_surface(surface, 0.0, 0.0);
            cr.source().set_filter(cairo::Filter::Good);
            let _ = cr.paint();
            let _ = cr.restore();
        }

        cr.rectangle(l.x + 0.5, l.y + 0.5, w - 1.0, h - 1.0);
        cr.set_source_rgba(r, g, b, 0.35);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        if let Some(sel) = self.selection.get() {
            let sx = l.x + sel.x as f64 * l.scale;
            let sy = l.y + sel.y as f64 * l.scale;
            let sw = sel.width as f64 * l.scale;
            let sh = sel.height as f64 * l.scale;

            // Darken what will not be scanned.
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            cr.rectangle(l.x, l.y, w, h);
            cr.rectangle(sx, sy, sw, sh);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
            let _ = cr.fill();
            cr.set_fill_rule(cairo::FillRule::Winding);

            let accent = adw::StyleManager::default().accent_color_rgba();
            cr.rectangle(sx + 1.0, sy + 1.0, (sw - 2.0).max(0.0), (sh - 2.0).max(0.0));
            cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, 1.0);
            cr.set_line_width(2.0);
            let _ = cr.stroke();
        }

        // Mark of the corner where the original is aligned (top left).
        cr.move_to(l.x, l.y + 18.0);
        cr.line_to(l.x, l.y);
        cr.line_to(l.x + 18.0, l.y);
        cr.set_source_rgba(0.91, 0.33, 0.13, 1.0);
        cr.set_line_width(3.0);
        let _ = cr.stroke();
    }
}

fn rect(a: (f64, f64), b: (f64, f64)) -> Region {
    Region {
        x: a.0.min(b.0).round() as u32,
        y: a.1.min(b.1).round() as u32,
        width: (a.0 - b.0).abs().round() as u32,
        height: (a.1 - b.1).abs().round() as u32,
    }
}

/// Converts an RGBA image into a Cairo surface (premultiplied ARGB32, BGRA order).
fn surface_from_rgba(image: &image::RgbaImage) -> Option<cairo::ImageSurface> {
    let (width, height) = image.dimensions();
    let stride = cairo::Format::ARgb32.stride_for_width(width).ok()? as usize;
    let mut data = vec![0u8; stride * height as usize];
    for (y, row) in image.rows().enumerate() {
        for (x, pixel) in row.enumerate() {
            let [r, g, b, a] = pixel.0;
            let premultiply = |c: u8| ((c as u16 * a as u16 + 127) / 255) as u8;
            let offset = y * stride + x * 4;
            data[offset..offset + 4].copy_from_slice(&[premultiply(b), premultiply(g), premultiply(r), a]);
        }
    }
    cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, width as i32, height as i32, stride as i32).ok()
}
