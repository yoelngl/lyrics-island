//! Text shaping (cosmic-text) and glyph blitting into premultiplied tiny-skia pixmaps.

use std::ops::Range;

use cosmic_text::{
    Attrs, Buffer, Ellipsize, EllipsizeHeightLimit, FontSystem, Metrics, Shaping, SwashCache,
    SwashContent, Weight, Wrap,
};
use tiny_skia::Pixmap;

pub struct Text {
    fs: FontSystem,
    cache: SwashCache,
}

/// Font parameters in logical px.
#[derive(Clone, Copy, PartialEq)]
pub struct Style {
    pub size: f32,
    pub line: f32,
    pub weight: u16,
}

pub const fn style(size: f32, line: f32, weight: u16) -> Style {
    Style { size, line, weight }
}

/// Straight-alpha color, 0..1.
#[derive(Clone, Copy)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

impl Rgba {
    pub const fn rgba8(r: u8, g: u8, b: u8, a: f32) -> Self {
        Rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
    }

    pub fn alpha(self, a: f32) -> Self {
        Rgba(self.0, self.1, self.2, self.3 * a)
    }
}

/// Physical-pixel clip rectangle (x0, y0, x1, y1).
pub type Clip = (f32, f32, f32, f32);

/// A shaped piece of text; reshapes only when its text or layout parameters change.
pub struct Label {
    buf: Buffer,
    text: String,
    key: Option<(Style, u32, u32, usize)>,
}

impl Label {
    pub fn new() -> Self {
        Self { buf: Buffer::new_empty(Metrics::new(1.0, 1.0)), text: String::new(), key: None }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set(&mut self, s: &str) {
        if self.text != s {
            self.text.clear();
            self.text.push_str(s);
            self.key = None;
        }
    }

    /// Lays out at `scale` within `max_w` physical px, ellipsizing after `lines` lines (`0` = unlimited wrap).
    pub fn layout(&mut self, t: &mut Text, st: Style, scale: f32, max_w: f32, lines: usize) {
        let key = Some((st, scale.to_bits(), max_w.to_bits(), lines));
        if self.key == key {
            return;
        }
        self.key = key;
        let b = &mut self.buf;
        b.set_metrics(Metrics::new(st.size * scale, st.line * scale));
        b.set_wrap(if lines == 1 { Wrap::None } else { Wrap::WordOrGlyph });
        b.set_ellipsize(if lines == 0 {
            Ellipsize::None
        } else {
            Ellipsize::End(EllipsizeHeightLimit::Lines(lines))
        });
        b.set_size(Some(max_w.max(1.0)), None);
        b.set_text(&self.text, &Attrs::new().weight(Weight(st.weight)), Shaping::Advanced, None);
        b.shape_until_scroll(&mut t.fs, false);
    }

    /// Widest laid-out line in physical px.
    pub fn width(&self) -> f32 {
        self.buf.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
    }
}

impl Text {
    pub fn new() -> Self {
        Self { fs: FontSystem::new(), cache: SwashCache::new() }
    }

    /// Draws `label` with its top-left at `pos = (x, y)` physical px.
    /// With `karaoke = Some((word, frac))` only the sung part is drawn: glyphs before `word`
    /// fully, glyphs of `word` up to `frac` of its width.
    pub fn draw(
        &mut self,
        pm: &mut Pixmap,
        label: &Label,
        pos: (f32, f32),
        color: Rgba,
        clip: Clip,
        karaoke: Option<(Range<usize>, f32)>,
    ) {
        if color.3 <= 0.0 {
            return;
        }
        let (x, y) = pos;
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let cx0 = clip.0.max(0.0);
        let cy0 = clip.1.max(0.0) as i32;
        let cx1 = clip.2.min(pw as f32);
        let cy1 = (clip.3.min(ph as f32)).ceil() as i32;
        let data = pm.data_mut();
        for run in label.buf.layout_runs() {
            let cut = karaoke.as_ref().map(|(word, frac)| {
                let (a, b) = run
                    .glyphs
                    .iter()
                    .filter(|g| g.start >= word.start && g.start < word.end)
                    .fold((f32::MAX, f32::MIN), |(a, b), g| (a.min(g.x), b.max(g.x + g.w)));
                if a <= b { x + a + frac * (b - a) } else { f32::MAX }
            });
            for g in run.glyphs {
                let limit = match &karaoke {
                    None => cx1,
                    Some((word, _)) if g.end <= word.start => cx1,
                    Some((word, _)) if g.start < word.end => cut.unwrap_or(cx1).min(cx1),
                    Some(_) => continue,
                };
                let pg = g.physical((x, y + run.line_y), 1.0);
                let Some(img) = self.cache.get_image(&mut self.fs, pg.cache_key) else {
                    continue;
                };
                let p = img.placement;
                let gx = pg.x + p.left;
                let gy = pg.y - p.top;
                for row in 0..p.height as i32 {
                    let py = gy + row;
                    if py < cy0 || py >= cy1 {
                        continue;
                    }
                    for col in 0..p.width as i32 {
                        let px = gx + col;
                        // Fractional coverage at the clip edges keeps the karaoke wipe smooth.
                        let cover = ((px as f32 + 1.0 - cx0).min(1.0)).min(limit - px as f32).clamp(0.0, 1.0);
                        if cover <= 0.0 || px < 0 || px >= pw {
                            continue;
                        }
                        let i = (row * p.width as i32 + col) as usize;
                        let (r, gg, b, a) = match img.content {
                            SwashContent::Mask => {
                                (color.0, color.1, color.2, img.data[i] as f32 / 255.0 * color.3)
                            }
                            SwashContent::SubpixelMask => {
                                let s = &img.data[i * 4..i * 4 + 3];
                                let m = (s[0] as f32 + s[1] as f32 + s[2] as f32) / 765.0;
                                (color.0, color.1, color.2, m * color.3)
                            }
                            SwashContent::Color => {
                                let s = &img.data[i * 4..i * 4 + 4];
                                let k = 1.0 / 255.0;
                                (s[0] as f32 * k, s[1] as f32 * k, s[2] as f32 * k, s[3] as f32 * k * color.3)
                            }
                        };
                        blend(&mut data[((py * pw + px) * 4) as usize..][..4], r, gg, b, a * cover);
                    }
                }
            }
        }
    }
}

/// Source-over of a straight-alpha color onto a premultiplied RGBA8 pixel.
fn blend(d: &mut [u8], r: f32, g: f32, b: f32, a: f32) {
    if a <= 0.0 {
        return;
    }
    let k = 1.0 - a;
    d[0] = (r * a * 255.0 + d[0] as f32 * k + 0.5) as u8;
    d[1] = (g * a * 255.0 + d[1] as f32 * k + 0.5) as u8;
    d[2] = (b * a * 255.0 + d[2] as f32 * k + 0.5) as u8;
    d[3] = (a * 255.0 + d[3] as f32 * k + 0.5) as u8;
}
