//! Desktop lyrics: a wide transparent overlay with the current line large and the next one below.

use std::ops::Range;

use tiny_skia::{Color, Pixmap};

use super::text::{style, Clip, Label, Rgba, Style, Text};

/// Logical padding around the text; leaves room for the outline.
pub const PAD: f32 = 8.0;
const NEXT_RATIO: f32 = 0.6;
const LINE_RATIO: f32 = 1.3;

fn cur_style(font: f32) -> Style {
    style(font, font * LINE_RATIO, 700)
}

fn next_style(font: f32) -> Style {
    style(font * NEXT_RATIO, font * NEXT_RATIO * LINE_RATIO, 600)
}

/// Logical window height for a font size.
pub fn height(font: f32) -> f32 {
    PAD * 2.0 + font * LINE_RATIO * (1.0 + NEXT_RATIO)
}

pub struct Desktop {
    pub cur: Label,
    pub next: Label,
}

impl Default for Desktop {
    fn default() -> Self {
        Self::new()
    }
}

impl Desktop {
    pub fn new() -> Self {
        Self { cur: Label::new(), next: Label::new() }
    }

    pub fn render(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        scale: f32,
        font: f32,
        hittable: bool,
        karaoke: Option<(&str, Range<usize>, f32)>,
    ) {
        if hittable {
            // Nearly invisible fill: layered/transparent windows pass clicks through alpha-0 pixels.
            pm.fill(Color::from_rgba8(0, 0, 0, 1));
        }
        let (w, h) = (pm.width() as f32, pm.height() as f32);
        let pad = PAD * scale;
        let clip = (0.0, 0.0, w, h);
        let shadow = Rgba(0.0, 0.0, 0.0, 0.45);

        self.cur.layout(t, cur_style(font), scale, w - 2.0 * pad, 1);
        let x = ((w - self.cur.width()) / 2.0).round();
        outline(t, pm, &self.cur, (x, pad), 1.6 * scale, shadow, clip);
        match karaoke {
            Some((line, word, frac)) if self.cur.text() == line => {
                t.draw(pm, &self.cur, (x, pad), Rgba(1.0, 1.0, 1.0, 0.5), clip, None);
                t.draw(pm, &self.cur, (x, pad), Rgba(1.0, 1.0, 1.0, 1.0), clip, Some((word, frac)));
            }
            _ => t.draw(pm, &self.cur, (x, pad), Rgba(1.0, 1.0, 1.0, 1.0), clip, None),
        }

        self.next.layout(t, next_style(font), scale, w - 2.0 * pad, 1);
        let x = ((w - self.next.width()) / 2.0).round();
        let y = pad + font * LINE_RATIO * scale;
        outline(t, pm, &self.next, (x, y), 1.2 * scale, shadow, clip);
        t.draw(pm, &self.next, (x, y), Rgba(1.0, 1.0, 1.0, 0.75), clip, None);
    }
}

/// Dark halo in eight directions so the text reads on any background.
fn outline(t: &mut Text, pm: &mut Pixmap, l: &Label, pos: (f32, f32), r: f32, c: Rgba, clip: Clip) {
    let (x, y) = pos;
    for (dx, dy) in [(-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (0.0, 1.0), (1.0, 1.0)] {
        t.draw(pm, l, (x + dx * r, y + dy * r), c, clip, None);
    }
}
