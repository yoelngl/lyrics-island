//! The island: compact pill with the current line, expanding into an interactive now-playing card.

use std::ops::Range;
use std::time::{Duration, Instant};

use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LinearGradient, Mask, Paint, PathBuilder, Pattern,
    Pixmap, Point, Shader, SpreadMode, Stroke, Transform,
};

use super::text::{style, Label, Rgba, Style, Text};
use super::{ease_in_out_quad, ease_out_quart, ease_out_quad, ease_out_spring, rrect, Anim};
use crate::model::Command;

pub const PILL_H: f32 = 34.0;
pub const PILL_MIN_W: f32 = 184.0;
pub const EXPANDED_W: f32 = 416.0;
pub const EXPANDED_H: f32 = 286.0;

const COMPACT_PAD_L: f32 = 14.0;
const COMPACT_PAD_R: f32 = 16.0;
const COMPACT_GAP: f32 = 10.0;
const BARS_W: f32 = 18.0;
const CARD_PAD_X: f32 = 24.0;

const BUTTONS: [(f32, Command); 3] = [
    (EXPANDED_W / 2.0 - 64.0, Command::Previous),
    (EXPANDED_W / 2.0, Command::PlayPause),
    (EXPANDED_W / 2.0 + 64.0, Command::Next),
];
const BUTTON_Y: f32 = 230.0;
const BUTTON_R: f32 = 18.0;
/// 44px diameter hit target around each transport button, leaving 20px clear gap between neighbors.
const BUTTON_HIT_R: f32 = 22.0;
const DEFAULT_TRACK_X: (f32, f32) = (62.0, EXPANDED_W - 62.0);
/// Settings gear center in the expanded card's top-right corner, logical px from the card origin.
const GEAR: (f32, f32) = (EXPANDED_W - CARD_PAD_X - 10.0, 30.0);

/// Right-click menu entries, top to bottom.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuItem {
    Hide,
    Lock,
    Desktop,
    ClickThrough,
    Karaoke,
    Settings,
    Quit,
}
pub const MENU: [(MenuItem, &str); 7] = [
    (MenuItem::Hide, "Hide lyrics"),
    (MenuItem::Lock, "Lock position"),
    (MenuItem::Desktop, "Desktop lyrics"),
    (MenuItem::ClickThrough, "Click-through"),
    (MenuItem::Karaoke, "Karaoke highlight"),
    (MenuItem::Settings, "Settings\u{2026}"),
    (MenuItem::Quit, "Quit"),
];
const MENU_W: f32 = 232.0;
const MENU_PAD: f32 = 8.0;
const MENU_ROW_H: f32 = 34.0;
const MENU_H: f32 = MENU_PAD * 2.0 + MENU_ROW_H * MENU.len() as f32;

const TITLE: Style = style(14.67, 18.0, 700);
const ARTIST: Style = style(13.33, 17.0, 400);
const SIDE: Style = style(13.33, 18.0, 400);
const CUR: Style = style(17.33, 22.0, 700);
const TIME: Style = style(11.33, 14.0, 400);
const CREDIT: Style = style(10.67, 14.0, 400);
const MENU_LABEL: Style = style(13.33, 18.0, 400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Button(Command),
    Progress,
    PrevLine,
    NextLine,
    /// Gear in the expanded card: opens the Settings window.
    Settings,
    Menu(MenuItem),
}

/// Logical height of the compact pill for a point size, never shorter than `PILL_H`.
pub fn pill_height(font_pt: f32) -> f32 {
    (font_pt * (96.0 / 72.0) * 1.35 + 14.0).round().max(PILL_H)
}

/// Logical square side of the compact pill album art, leaving generous vertical and horizontal inset.
pub fn compact_art_size(font_pt: f32) -> f32 {
    (pill_height(font_pt) - 12.0).max(20.0)
}

/// Compresses raw album-art colors into a dark or light luminance band so foreground text
/// maintains WCAG AA contrast across every possible cover image.
pub fn tone_art_gradient(a: [u8; 3], b: [u8; 3], light: bool) -> ([u8; 3], [u8; 3]) {
    let tone = |c: [u8; 3]| {
        c.map(|v| {
            if light {
                (255.0 - (255.0 - v as f32) * 0.22).round() as u8
            } else {
                (v as f32 * 0.28).round() as u8
            }
        })
    };
    (tone(a), tone(b))
}

#[derive(Clone, Copy)]
struct Palette {
    primary: Rgba,
    secondary: Rgba,
    karaoke_base: Rgba,
    line_hover_bg: Rgba,
    track_bg: Rgba,
    btn_hover_bg: Rgba,
    btn_press_bg: Rgba,
    focus_ring: Rgba,
    border: Rgba,
}

impl Palette {
    fn for_theme(light: bool) -> Self {
        if light {
            Self {
                primary: Rgba::rgba8(17, 17, 19, 1.0),
                secondary: Rgba::rgba8(42, 42, 48, 0.92),
                karaoke_base: Rgba::rgba8(46, 46, 52, 0.80),
                line_hover_bg: Rgba(0.0, 0.0, 0.0, 0.07),
                track_bg: Rgba(0.0, 0.0, 0.0, 0.45),
                btn_hover_bg: Rgba(0.0, 0.0, 0.0, 0.09),
                btn_press_bg: Rgba(0.0, 0.0, 0.0, 0.16),
                focus_ring: Rgba::rgba8(0x1D, 0x4E, 0xD8, 1.0),
                border: Rgba(0.0, 0.0, 0.0, 0.16),
            }
        } else {
            Self {
                primary: Rgba(1.0, 1.0, 1.0, 1.0),
                secondary: Rgba::rgba8(226, 226, 232, 0.86),
                karaoke_base: Rgba::rgba8(226, 226, 232, 0.72),
                line_hover_bg: Rgba(1.0, 1.0, 1.0, 0.10),
                track_bg: Rgba(1.0, 1.0, 1.0, 0.42),
                btn_hover_bg: Rgba(1.0, 1.0, 1.0, 0.14),
                btn_press_bg: Rgba(1.0, 1.0, 1.0, 0.24),
                focus_ring: Rgba::rgba8(0x60, 0xA5, 0xFA, 1.0),
                border: Rgba(1.0, 1.0, 1.0, 0.14),
            }
        }
    }
}

/// Per-frame inputs owned by the Ui.
pub struct Ctx<'a> {
    pub now: Instant,
    pub scale: f32,
    pub light: bool,
    pub font_pt: f32,
    pub playing: bool,
    /// Horizontal center of the compact pill inside the canvas, in physical px.
    pub center_x: f32,
    /// True when the pill sits near the bottom of the screen so the card expands upward.
    pub expand_up: bool,
    /// Seconds since the Ui started; drives the gradient sweep.
    pub clock: f32,
    pub pos_ms: u64,
    pub len_ms: u64,
    /// Active drag fraction (0.0..=1.0) when scrubbing the progress bar.
    pub scrub_frac: Option<f32>,
    pub has_prev_line: bool,
    pub has_next_line: bool,
    /// Sung part of the current line and the text it applies to.
    pub karaoke: Option<(&'a str, Range<usize>, f32)>,
    pub bars: [f32; 4],
    pub art_small: Option<&'a Pixmap>,
    pub art_large: Option<&'a Pixmap>,
    /// Raw untoned album-art colors; toned inside `render` using `tone_art_gradient(..., c.light)`.
    pub gradient: Option<([u8; 3], [u8; 3])>,
    /// Check marks for `MENU` rows, same order.
    pub menu_checked: [bool; MENU.len()],
}

/// A label whose text changes slide out up and fade in from below (island.js _swapText).
pub struct Swap {
    pub label: Label,
    pending: Option<String>,
    t0: Option<Instant>,
}

const SWAP_OUT: Duration = Duration::from_millis(100);
const SWAP_IN: Duration = Duration::from_millis(180);

impl Swap {
    fn new() -> Self {
        Self { label: Label::new(), pending: None, t0: None }
    }

    /// Returns true when the text was applied right away.
    pub fn set(&mut self, s: &str, animate: bool, now: Instant) -> bool {
        if !animate {
            self.label.set(s);
            self.pending = None;
            self.t0 = None;
            return true;
        }
        if let Some(p) = &mut self.pending {
            p.clear();
            p.push_str(s);
        } else if self.label.text() != s {
            self.pending = Some(s.to_owned());
            self.t0 = Some(now);
        }
        false
    }

    /// Advances the swap; returns true when the pending text was just applied.
    pub fn step(&mut self, now: Instant) -> bool {
        let Some(t0) = self.t0 else { return false };
        if let Some(p) = &self.pending {
            if now >= t0 + SWAP_OUT {
                self.label.set(p);
                self.pending = None;
                self.t0 = Some(t0 + SWAP_OUT);
                return true;
            }
        } else if now >= t0 + SWAP_IN {
            self.t0 = None;
        }
        false
    }

    pub fn animating(&self) -> bool {
        self.t0.is_some()
    }

    pub fn settled(&self) -> bool {
        self.pending.is_none()
    }

    /// (alpha, y offset in logical px).
    fn look(&self, now: Instant) -> (f32, f32) {
        let Some(t0) = self.t0 else { return (1.0, 0.0) };
        let e = now.saturating_duration_since(t0).as_secs_f32();
        if self.pending.is_some() {
            let p = ease_out_quad((e / SWAP_OUT.as_secs_f32()).min(1.0));
            (1.0 - p, -5.0 * p)
        } else {
            let p = ease_out_quad((e / SWAP_IN.as_secs_f32()).min(1.0));
            (p, 5.0 * (1.0 - p))
        }
    }
}

/// The four "now playing" bars; values are 0..1 of the available height.
pub struct Bars {
    v: [Anim; 4],
    next: Instant,
    on: bool,
    seed: u32,
}

impl Bars {
    pub fn new(now: Instant) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.subsec_nanos())
            | 1;
        Self { v: [Anim::new(0.0, now); 4], next: now, on: false, seed }
    }

    pub fn step(&mut self, now: Instant, playing: bool) {
        if playing {
            if now >= self.next {
                for a in &mut self.v {
                    self.seed ^= self.seed << 13;
                    self.seed ^= self.seed >> 17;
                    self.seed ^= self.seed << 5;
                    a.set((self.seed % 1000) as f32 / 999.0, 170, 0, ease_in_out_quad, now);
                }
                self.next = now + Duration::from_millis(180);
            }
            self.on = true;
        } else if self.on {
            self.on = false;
            for a in &mut self.v {
                a.set(0.0, 200, 0, ease_out_quad, now);
            }
        }
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.on || self.v.iter().any(|a| !a.done(now))
    }

    pub fn values(&self, now: Instant) -> [f32; 4] {
        self.v.map(|a| a.value(now))
    }
}

pub struct Island {
    pub expanded: bool,
    /// The expanded shape shows the right-click menu instead of the now-playing card.
    pub menu: bool,
    /// Which compact-pill edge the expanded shape keeps fixed: 0.0 left (grows right), 0.5 center, 1.0 right.
    pub align: f32,
    menu_labels: [Label; MENU.len()],
    pub w: Anim,
    pub h: Anim,
    compact_a: Anim,
    expanded_a: Anim,
    pub line: Swap,
    pub cur: Swap,
    pub prev: Label,
    pub next: Label,
    pub title: Label,
    pub artist: Label,
    pub elapsed: Label,
    pub total: Label,
    pub credit: Label,
    scrub_elapsed: Label,
    pub hover: Option<Hit>,
    pub pressed: Option<Hit>,
    pub focus: Option<Hit>,
    pub left_at: Option<Instant>,
    /// Logical width the compact pill wants for its current line.
    pub pill_w: f32,
    /// Logical height of the compact pill for its current font size.
    pub pill_h: f32,
    /// Last laid-out logical `(x0, x1)` bounds of the progress bar track.
    track_x: (f32, f32),
    /// Reused per-frame clip mask so rendering never allocates on the heap.
    mask: Option<Mask>,
}

impl Island {
    pub fn new(now: Instant) -> Self {
        Self {
            expanded: false,
            menu: false,
            align: 0.5,
            menu_labels: std::array::from_fn(|i| {
                let mut l = Label::new();
                l.set(MENU[i].1);
                l
            }),
            w: Anim::new(PILL_MIN_W, now),
            h: Anim::new(PILL_H, now),
            compact_a: Anim::new(1.0, now),
            expanded_a: Anim::new(0.0, now),
            line: Swap::new(),
            cur: Swap::new(),
            prev: Label::new(),
            next: Label::new(),
            title: Label::new(),
            artist: Label::new(),
            elapsed: Label::new(),
            total: Label::new(),
            credit: Label::new(),
            scrub_elapsed: Label::new(),
            hover: None,
            pressed: None,
            focus: None,
            left_at: None,
            pill_w: PILL_MIN_W,
            pill_h: PILL_H,
            track_x: DEFAULT_TRACK_X,
            mask: None,
        }
    }

    /// True while shape, cross-fade, or text-swap transitions are actively running (120 Hz path).
    pub fn transitioning(&self, now: Instant) -> bool {
        !(self.w.done(now) && self.h.done(now) && self.compact_a.done(now) && self.expanded_a.done(now))
            || self.line.animating()
            || self.cur.animating()
    }

    /// Measures the compact line at `font_pt` and eases or jumps the pill to fit it.
    pub fn fit_pill(&mut self, t: &mut Text, scale: f32, font_pt: f32, max_w: f32, now: Instant, animate: bool) {
        let line_px = font_pt * (96.0 / 72.0);
        let line_h = (line_px * 1.35).round();
        let ph = pill_height(font_pt);
        let art = compact_art_size(font_pt);
        let chrome = COMPACT_PAD_L + art + COMPACT_GAP + COMPACT_GAP + BARS_W + COMPACT_PAD_R;
        let max_w = max_w.max(PILL_MIN_W);
        self.line.label.layout(t, style(line_px, line_h, 600), scale, (max_w - chrome).max(1.0) * scale, 1);
        self.pill_w = (self.line.label.width() / scale + chrome).clamp(PILL_MIN_W, max_w).round();
        self.pill_h = ph;
        if self.expanded {
            return;
        }
        if !animate {
            self.w.jump(self.pill_w);
            self.h.jump(self.pill_h);
            return;
        }
        if (self.pill_w - self.w.to).abs() < 3.0 && (self.pill_h - self.h.to).abs() < 0.5 {
            return;
        }
        self.w.set(self.pill_w, 240, 0, ease_out_quart, now);
        self.h.set(self.pill_h, 240, 0, ease_out_quart, now);
    }

    pub fn expand(&mut self, now: Instant) {
        self.open(now, false);
    }

    pub fn open_menu(&mut self, now: Instant) {
        self.open(now, true);
    }

    /// Logical size of the expanded shape for the current mode.
    fn card_size(&self) -> (f32, f32) {
        if self.menu {
            (MENU_W, MENU_H)
        } else {
            (EXPANDED_W, EXPANDED_H)
        }
    }

    /// Expands into the card or the menu; switching while expanded morphs and cross-fades the content.
    fn open(&mut self, now: Instant, menu: bool) {
        if self.expanded && self.menu == menu {
            return;
        }
        let switching = self.expanded;
        self.expanded = true;
        self.menu = menu;
        self.hover = None;
        self.pressed = None;
        self.focus = None;
        self.left_at = None;
        if switching {
            self.expanded_a.jump(0.0);
        } else {
            let text = self.cur.pending.take().unwrap_or_default();
            if !text.is_empty() {
                self.cur.label.set(&text);
            }
            self.cur.t0 = None;
            self.compact_a.set(0.0, 85, 0, ease_out_quad, now);
        }
        let (w, h) = self.card_size();
        self.w.set(w, 310, 0, ease_out_spring, now);
        self.h.set(h, 310, 0, ease_out_spring, now);
        self.expanded_a.set(1.0, 180, 55, ease_out_quad, now);
    }

    /// Left edge of a `w`-wide shape: keeps the compact pill's `align` edge fixed, clamped to the canvas.
    fn box_x(&self, win_w: f32, center_x: f32, scale: f32, w: f32) -> f32 {
        let pw = (self.pill_w * scale).max(1.0);
        (center_x - pw / 2.0 + (pw - w) * self.align).round().clamp(0.0, (win_w - w).max(0.0))
    }

    /// The caller re-runs fit_pill afterwards so the pill eases back to the current line.
    pub fn collapse(&mut self, now: Instant) {
        if !self.expanded {
            return;
        }
        self.expanded = false;
        if let Some(p) = self.line.pending.take() {
            self.line.label.set(&p);
        }
        self.line.t0 = None;
        self.hover = None;
        self.pressed = None;
        self.focus = None;
        self.left_at = None;
        self.expanded_a.set(0.0, 80, 0, ease_out_quad, now);
        self.compact_a.set(1.0, 160, 70, ease_out_quad, now);
    }

    /// Top-left `(ox, oy)` and size `(w, h)` of the morphing shape in physical px inside the fixed canvas.
    pub fn shape_rect(
        &self,
        win_size: (u32, u32),
        scale: f32,
        center_x: f32,
        expand_up: bool,
        now: Instant,
    ) -> (f32, f32, f32, f32) {
        let w = (self.w.value(now) * scale).max(1.0);
        let h = (self.h.value(now) * scale).max(1.0);
        let ox = self.box_x(win_size.0 as f32, center_x, scale, w);
        let oy = box_y(win_size.1 as f32, expand_up, h);
        (ox, oy, w, h)
    }

    /// Stationary compact pill top-left `(cox, coy)` and size `(cw, ch)` in physical px.
    /// Does not change during expand/collapse so the small lyrics never shifts on screen.
    pub fn compact_rect(
        &self,
        win_size: (u32, u32),
        scale: f32,
        center_x: f32,
        expand_up: bool,
    ) -> (f32, f32, f32, f32) {
        let cw = (self.pill_w * scale).max(1.0);
        let ch = (self.pill_h * scale).max(1.0);
        (self.box_x(win_size.0 as f32, center_x, scale, cw), box_y(win_size.1 as f32, expand_up, ch), cw, ch)
    }

    /// Expanded card origin `(eox, eoy)` in physical px.
    pub fn expanded_origin(&self, win_size: (u32, u32), scale: f32, center_x: f32, expand_up: bool) -> (f32, f32) {
        let (cw, ch) = self.card_size();
        let (ew, eh) = (cw * scale, ch * scale);
        (self.box_x(win_size.0 as f32, center_x, scale, ew), box_y(win_size.1 as f32, expand_up, eh))
    }

    /// Padded integer physical bounding box `(x, y, w, h)` of the active shape for input masking and dirty blits.
    pub fn bounds(
        &self,
        win_size: (u32, u32),
        scale: f32,
        center_x: f32,
        expand_up: bool,
        now: Instant,
    ) -> (i32, i32, u32, u32) {
        let (ox, oy, w, h) = self.shape_rect(win_size, scale, center_x, expand_up, now);
        let x0 = (ox.floor() as i32 - 2).clamp(0, win_size.0 as i32);
        let y0 = (oy.floor() as i32 - 2).clamp(0, win_size.1 as i32);
        let x1 = ((ox + w).ceil() as i32 + 2).clamp(x0, win_size.0 as i32);
        let y1 = ((oy + h).ceil() as i32 + 2).clamp(y0, win_size.1 as i32);
        (x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
    }

    /// Interactive control under `(x, y)` in logical px relative to the expanded card origin (expanded only).
    pub fn hit_at(&self, x: f32, y: f32, has_prev: bool, has_next: bool, has_len: bool) -> Option<Hit> {
        if !self.expanded {
            return None;
        }
        if self.menu {
            let row = ((y - MENU_PAD) / MENU_ROW_H).floor();
            if !(MENU_PAD..=MENU_W - MENU_PAD).contains(&x) || row < 0.0 {
                return None;
            }
            return MENU.get(row as usize).map(|&(item, _)| Hit::Menu(item));
        }
        if (x - GEAR.0).powi(2) + (y - GEAR.1).powi(2) <= BUTTON_HIT_R * BUTTON_HIT_R {
            return Some(Hit::Settings);
        }
        if let Some(&(_, cmd)) = BUTTONS
            .iter()
            .find(|(cx, _)| (x - cx).powi(2) + (y - BUTTON_Y).powi(2) <= BUTTON_HIT_R * BUTTON_HIT_R)
        {
            return Some(Hit::Button(cmd));
        }
        if has_len && (CARD_PAD_X..=EXPANDED_W - CARD_PAD_X).contains(&x) && (178.0..=206.0).contains(&y) {
            return Some(Hit::Progress);
        }
        if has_prev
            && !self.prev.text().is_empty()
            && (CARD_PAD_X - 6.0..=EXPANDED_W - CARD_PAD_X + 6.0).contains(&x)
            && (80.0..=105.0).contains(&y)
        {
            return Some(Hit::PrevLine);
        }
        if has_next
            && !self.next.text().is_empty()
            && (CARD_PAD_X - 6.0..=EXPANDED_W - CARD_PAD_X + 6.0).contains(&x)
            && (152.0..=177.0).contains(&y)
        {
            return Some(Hit::NextLine);
        }
        None
    }

    /// Maps logical `x` relative to the expanded card origin into `0.0..=1.0` along the progress track.
    pub fn progress_frac(&self, x: f32) -> f32 {
        let (x0, x1) = self.track_x;
        ((x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0)
    }

    /// Cycles keyboard focus forward (`Tab`) or backward (`Shift+Tab`) through active expanded controls.
    pub fn move_focus(&mut self, reverse: bool, has_prev: bool, has_next: bool, has_len: bool) {
        let mut items = Vec::with_capacity(MENU.len());
        if self.menu {
            items.extend(MENU.iter().map(|&(item, _)| Hit::Menu(item)));
        }
        if has_prev && !self.menu {
            items.push(Hit::PrevLine);
        }
        if has_next && !self.menu {
            items.push(Hit::NextLine);
        }
        if has_len && !self.menu {
            items.push(Hit::Progress);
        }
        if !self.menu {
            items.extend(BUTTONS.iter().map(|&(_, cmd)| Hit::Button(cmd)));
            items.push(Hit::Settings);
        }
        let next = match self.focus.and_then(|cur| items.iter().position(|&item| item == cur)) {
            Some(i) if reverse => items[(i + items.len() - 1) % items.len()],
            Some(i) => items[(i + 1) % items.len()],
            None if reverse => items[items.len() - 1],
            None => items[0],
        };
        self.focus = Some(next);
    }

    pub fn render(&mut self, pm: &mut Pixmap, t: &mut Text, c: &Ctx) {
        let (s, now) = (c.scale, c.now);
        let win_size = (pm.width(), pm.height());
        let (ox, oy, w, h) = self.shape_rect(win_size, s, c.center_x, c.expand_up, now);
        let radius = (h / 2.0).min(28.0 * s);
        let Some(shape) = rrect(ox, oy, w, h, radius) else { return };
        let pal = Palette::for_theme(c.light);

        let mut paint = Paint::default();
        let bg_alpha = if c.light { 244 } else { 240 };
        match c.gradient {
            Some((raw_a, raw_b)) => {
                let (a, b) = tone_art_gradient(raw_a, raw_b, c.light);
                let col = |k: f32| {
                    let m = |i: usize| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * k).round() as u8;
                    Color::from_rgba8(m(0), m(1), m(2), bg_alpha)
                };
                let k = (c.clock * std::f32::consts::TAU / 8.0).sin() / 2.0 + 0.5;
                paint.shader = LinearGradient::new(
                    Point::from_xy(ox, oy),
                    Point::from_xy(ox + w, oy),
                    vec![GradientStop::new(0.0, col(k)), GradientStop::new(1.0, col(1.0 - k))],
                    SpreadMode::Pad,
                    Transform::identity(),
                )
                .unwrap_or(Shader::SolidColor(col(k)));
            }
            None if c.light => paint.set_color_rgba8(255, 255, 255, bg_alpha),
            None => paint.set_color_rgba8(14, 14, 16, bg_alpha),
        }
        pm.fill_path(&shape, &paint, FillRule::Winding, Transform::identity(), None);

        let ca = self.compact_a.value(now);
        if ca > 0.01 {
            // Render compact content at its stationary compact box so small lyrics never moves when expanding/collapsing.
            let crect = self.compact_rect(win_size, s, c.center_x, c.expand_up);
            self.render_compact(pm, t, c, crect, ca, pal);
        }
        let ea = self.expanded_a.value(now);
        if ea > 0.01 {
            let eorig = self.expanded_origin(win_size, s, c.center_x, c.expand_up);
            if self.menu {
                self.render_menu(pm, t, c, eorig, ea, pal);
            } else {
                self.render_expanded(pm, t, c, eorig, ea, pal);
            }
        }

        // Reuse the cached mask and apply it strictly over the active shape's bounding rows/cols.
        let (bx, by, bw, bh) = self.bounds(win_size, s, c.center_x, c.expand_up, now);
        if self.mask.as_ref().is_none_or(|m| m.width() != win_size.0 || m.height() != win_size.1) {
            self.mask = Mask::new(win_size.0, win_size.1);
        }
        if let Some(mask) = self.mask.as_mut() {
            let pw = win_size.0 as usize;
            let (x0, y0, x1, y1) = (bx as usize, by as usize, (bx as usize + bw as usize).min(pw), (by as usize + bh as usize).min(win_size.1 as usize));
            let mdata = mask.data_mut();
            for row in y0..y1 {
                mdata[row * pw + x0..row * pw + x1].fill(0);
            }
            mask.fill_path(&shape, FillRule::Winding, true, Transform::identity());
            let mdata = mask.data();
            let pdata = pm.data_mut();
            for row in y0..y1 {
                for col in x0..x1 {
                    let i = row * pw + col;
                    let ma = mdata[i] as u32;
                    if ma < 255 {
                        let px = &mut pdata[i * 4..i * 4 + 4];
                        if ma == 0 {
                            px.fill(0);
                        } else {
                            px[0] = ((px[0] as u32 * ma + 127) / 255) as u8;
                            px[1] = ((px[1] as u32 * ma + 127) / 255) as u8;
                            px[2] = ((px[2] as u32 * ma + 127) / 255) as u8;
                            px[3] = ((px[3] as u32 * ma + 127) / 255) as u8;
                        }
                    }
                }
            }
        }

        let inset = 0.5 * s;
        stroke(
            pm,
            rrect(ox + inset, oy + inset, w - 2.0 * inset, h - 2.0 * inset, (radius - inset).max(0.0)),
            1.0 * s,
            pal.border,
        );
    }

    fn render_compact(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        c: &Ctx,
        crect: (f32, f32, f32, f32),
        a: f32,
        pal: Palette,
    ) {
        let (cox, coy, cw, ch) = crect;
        let s = c.scale;
        let art = compact_art_size(c.font_pt);
        draw_art(
            pm,
            c.art_small,
            (cox + COMPACT_PAD_L * s, coy + (ch - art * s) / 2.0, art * s),
            6.0 * s,
            a,
            c.light,
        );

        let line_px = c.font_pt * (96.0 / 72.0);
        let line_h = (line_px * 1.35).round();
        let (la, dy) = self.line.look(c.now);
        let x = cox + (COMPACT_PAD_L + art + COMPACT_GAP) * s;
        let right = cox + cw - (COMPACT_PAD_R + BARS_W + COMPACT_GAP) * s;
        let y = coy + (ch - line_h * s) / 2.0 + dy * s;
        let clip = (x, coy, right, coy + ch);
        match &c.karaoke {
            Some((line, word, frac)) if self.line.settled() && self.line.label.text() == *line => {
                t.draw(pm, &self.line.label, (x, y), pal.karaoke_base.alpha(a * la), clip, None);
                t.draw(pm, &self.line.label, (x, y), pal.primary.alpha(a * la), clip, Some((word.clone(), *frac)));
            }
            _ => t.draw(pm, &self.line.label, (x, y), pal.primary.alpha(a * la), clip, None),
        }

        let bar_h = (pill_height(c.font_pt) * 0.48).round().min(18.0);
        draw_bars(
            pm,
            c.bars,
            (cox + cw - (COMPACT_PAD_R + BARS_W) * s, coy + ch / 2.0),
            bar_h,
            s,
            pal.primary.alpha(a),
        );
    }

    fn render_expanded(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        c: &Ctx,
        orig: (f32, f32),
        a: f32,
        pal: Palette,
    ) {
        let (eox, eoy) = orig;
        let s = c.scale;
        let x = |v: f32| eox + v * s;
        let y = |v: f32| eoy + v * s;
        let clip = (x(CARD_PAD_X), eoy, x(EXPANDED_W - CARD_PAD_X), eoy + EXPANDED_H * s);

        draw_art(pm, c.art_large, (x(CARD_PAD_X), y(18.0), 56.0 * s), 12.0 * s, a, c.light);
        let meta_x = CARD_PAD_X + 56.0 + 12.0;
        // Bars sit left of the gear; the gear's 44px target spans 22px either side of GEAR.0.
        let bars_x = GEAR.0 - 22.0 - BARS_W;
        let meta_w = (bars_x - 12.0 - meta_x) * s;
        self.title.layout(t, TITLE, s, meta_w, 1);
        self.artist.layout(t, ARTIST, s, meta_w, 1);
        t.draw(pm, &self.title, (x(meta_x), y(28.5)), pal.primary.alpha(a), clip, None);
        t.draw(pm, &self.artist, (x(meta_x), y(46.5)), pal.secondary.alpha(a), clip, None);
        draw_bars(pm, c.bars, (x(bars_x), y(46.0)), 20.0, s, pal.primary.alpha(a));

        let (gx, gy) = (x(GEAR.0), y(GEAR.1));
        if self.pressed == Some(Hit::Settings) {
            fill(pm, PathBuilder::from_circle(gx, gy, BUTTON_R * s), pal.btn_press_bg.alpha(a));
        } else if self.hover == Some(Hit::Settings) {
            fill(pm, PathBuilder::from_circle(gx, gy, BUTTON_R * s), pal.btn_hover_bg.alpha(a));
        }
        stroke(pm, gear_icon(gx, gy, s), 1.6 * s, pal.primary.alpha(a));
        if self.focus == Some(Hit::Settings) {
            stroke(pm, PathBuilder::from_circle(gx, gy, (BUTTON_R + 2.0) * s), 2.0 * s, pal.focus_ring.alpha(a));
        }

        let text_w = (EXPANDED_W - 2.0 * CARD_PAD_X) * s;
        self.prev.layout(t, SIDE, s, text_w, 1);
        self.next.layout(t, SIDE, s, text_w, 1);
        self.cur.label.layout(t, CUR, s, text_w, 2);

        let can_prev = c.has_prev_line && !self.prev.text().is_empty();
        if can_prev && self.hover == Some(Hit::PrevLine) {
            fill(
                pm,
                rrect(x(CARD_PAD_X - 6.0), y(82.0), (EXPANDED_W - 2.0 * (CARD_PAD_X - 6.0)) * s, 24.0 * s, 6.0 * s),
                pal.line_hover_bg.alpha(a),
            );
        }
        let prev_col = if can_prev && (self.hover == Some(Hit::PrevLine) || self.focus == Some(Hit::PrevLine)) {
            pal.primary
        } else {
            pal.secondary
        };
        t.draw(pm, &self.prev, (x(CARD_PAD_X), y(86.0)), prev_col.alpha(a), clip, None);
        if can_prev && self.focus == Some(Hit::PrevLine) {
            stroke(
                pm,
                rrect(x(CARD_PAD_X - 6.0), y(82.0), (EXPANDED_W - 2.0 * (CARD_PAD_X - 6.0)) * s, 24.0 * s, 6.0 * s),
                2.0 * s,
                pal.focus_ring.alpha(a),
            );
        }

        let (la, dy) = self.cur.look(c.now);
        let cy = y(108.0 + dy);
        match &c.karaoke {
            Some((line, word, frac)) if self.cur.settled() && self.cur.label.text() == *line => {
                t.draw(pm, &self.cur.label, (x(CARD_PAD_X), cy), pal.karaoke_base.alpha(a * la), clip, None);
                t.draw(
                    pm,
                    &self.cur.label,
                    (x(CARD_PAD_X), cy),
                    pal.primary.alpha(a * la),
                    clip,
                    Some((word.clone(), *frac)),
                );
            }
            _ => t.draw(pm, &self.cur.label, (x(CARD_PAD_X), cy), pal.primary.alpha(a * la), clip, None),
        }

        let can_next = c.has_next_line && !self.next.text().is_empty();
        if can_next && self.hover == Some(Hit::NextLine) {
            fill(
                pm,
                rrect(x(CARD_PAD_X - 6.0), y(152.0), (EXPANDED_W - 2.0 * (CARD_PAD_X - 6.0)) * s, 24.0 * s, 6.0 * s),
                pal.line_hover_bg.alpha(a),
            );
        }
        let next_col = if can_next && (self.hover == Some(Hit::NextLine) || self.focus == Some(Hit::NextLine)) {
            pal.primary
        } else {
            pal.secondary
        };
        t.draw(pm, &self.next, (x(CARD_PAD_X), y(156.0)), next_col.alpha(a), clip, None);
        if can_next && self.focus == Some(Hit::NextLine) {
            stroke(
                pm,
                rrect(x(CARD_PAD_X - 6.0), y(152.0), (EXPANDED_W - 2.0 * (CARD_PAD_X - 6.0)) * s, 24.0 * s, 6.0 * s),
                2.0 * s,
                pal.focus_ring.alpha(a),
            );
        }

        self.elapsed.layout(t, TIME, s, 80.0 * s, 1);
        self.total.layout(t, TIME, s, 80.0 * s, 1);
        let tw = self.total.width();
        let (ew, frac) = match c.scrub_frac {
            Some(sf) => {
                let clamped = sf.clamp(0.0, 1.0);
                let preview_ms = (clamped * c.len_ms as f32).round() as u64;
                self.scrub_elapsed.set(&fmt_time(preview_ms));
                self.scrub_elapsed.layout(t, TIME, s, 80.0 * s, 1);
                t.draw(pm, &self.scrub_elapsed, (x(CARD_PAD_X), y(186.0)), pal.primary.alpha(a), clip, None);
                (self.elapsed.width().max(self.scrub_elapsed.width()), clamped)
            }
            None => {
                t.draw(pm, &self.elapsed, (x(CARD_PAD_X), y(186.0)), pal.secondary.alpha(a), clip, None);
                let f = if c.len_ms > 0 { (c.pos_ms as f32 / c.len_ms as f32).clamp(0.0, 1.0) } else { 0.0 };
                (self.elapsed.width(), f)
            }
        };
        t.draw(pm, &self.total, (x(EXPANDED_W - CARD_PAD_X) - tw, y(186.0)), pal.secondary.alpha(a), clip, None);

        let lx0 = CARD_PAD_X + ew / s + 10.0;
        let lx1 = (EXPANDED_W - CARD_PAD_X - tw / s - 10.0).max(lx0 + 1.0);
        if c.scrub_frac.is_none() {
            self.track_x = (lx0, lx1);
        }
        let (tx0, tx1) = (x(lx0), x(lx1));
        let can_seek = c.len_ms > 0;
        let prog_active = can_seek
            && (self.hover == Some(Hit::Progress)
                || self.focus == Some(Hit::Progress)
                || c.scrub_frac.is_some());
        let th = if prog_active { 6.0 * s } else { 4.0 * s };
        let tcy = y(193.0);
        fill(pm, rrect(tx0, tcy - th / 2.0, tx1 - tx0, th, th / 2.0), pal.track_bg.alpha(a));
        fill(pm, rrect(tx0, tcy - th / 2.0, (tx1 - tx0) * frac, th, th / 2.0), pal.primary.alpha(a));
        if prog_active {
            let thumb_x = tx0 + (tx1 - tx0) * frac;
            fill(pm, PathBuilder::from_circle(thumb_x, tcy, 6.0 * s), pal.primary.alpha(a));
        }
        if can_seek && self.focus == Some(Hit::Progress) {
            stroke(
                pm,
                rrect(tx0 - 8.0 * s, y(183.0), (tx1 - tx0) + 16.0 * s, 20.0 * s, 10.0 * s),
                2.0 * s,
                pal.focus_ring.alpha(a),
            );
        }

        for (bx, cmd) in BUTTONS {
            let hit = Hit::Button(cmd);
            let (cx, cy) = (x(bx), y(BUTTON_Y));
            if self.pressed == Some(hit) {
                fill(pm, PathBuilder::from_circle(cx, cy, BUTTON_R * s), pal.btn_press_bg.alpha(a));
            } else if self.hover == Some(hit) {
                fill(pm, PathBuilder::from_circle(cx, cy, BUTTON_R * s), pal.btn_hover_bg.alpha(a));
            }
            let icon = match cmd {
                Command::Previous => skip_icon(cx, cy, s, -1.0),
                Command::Next => skip_icon(cx, cy, s, 1.0),
                Command::PlayPause if c.playing => pause_icon(cx, cy, s),
                Command::PlayPause => play_icon(cx, cy, s),
                Command::Seek(_) => None,
            };
            fill(pm, icon, pal.primary.alpha(a));
            if self.focus == Some(hit) {
                stroke(
                    pm,
                    PathBuilder::from_circle(cx, cy, (BUTTON_R + 2.0) * s),
                    2.0 * s,
                    pal.focus_ring.alpha(a),
                );
            }
        }

        self.credit.layout(t, CREDIT, s, text_w, 1);
        let cw = self.credit.width();
        t.draw(pm, &self.credit, (x(EXPANDED_W / 2.0) - cw / 2.0, y(258.0)), pal.secondary.alpha(a), clip, None);
    }

    fn render_menu(&mut self, pm: &mut Pixmap, t: &mut Text, c: &Ctx, orig: (f32, f32), a: f32, pal: Palette) {
        let (eox, eoy) = orig;
        let s = c.scale;
        let clip = (eox, eoy, eox + MENU_W * s, eoy + MENU_H * s);
        let row_w = (MENU_W - 2.0 * MENU_PAD) * s;
        for (i, &(item, _)) in MENU.iter().enumerate() {
            let hit = Hit::Menu(item);
            let (rx, ry) = (eox + MENU_PAD * s, eoy + (MENU_PAD + i as f32 * MENU_ROW_H) * s);
            let row = || rrect(rx, ry, row_w, MENU_ROW_H * s, 8.0 * s);
            if self.pressed == Some(hit) {
                fill(pm, row(), pal.btn_press_bg.alpha(a));
            } else if self.hover == Some(hit) {
                fill(pm, row(), pal.line_hover_bg.alpha(a));
            }
            if self.focus == Some(hit) {
                stroke(pm, row(), 2.0 * s, pal.focus_ring.alpha(a));
            }
            let label = &mut self.menu_labels[i];
            label.layout(t, MENU_LABEL, s, row_w - 44.0 * s, 1);
            let ty = ry + (MENU_ROW_H - MENU_LABEL.line) / 2.0 * s;
            t.draw(pm, label, (rx + 12.0 * s, ty), pal.primary.alpha(a), clip, None);
            if c.menu_checked[i] {
                let (cx, cy) = (rx + row_w - 20.0 * s, ry + MENU_ROW_H / 2.0 * s);
                let mut pb = PathBuilder::new();
                pb.move_to(cx - 5.0 * s, cy);
                pb.line_to(cx - 1.5 * s, cy + 3.5 * s);
                pb.line_to(cx + 5.0 * s, cy - 4.0 * s);
                stroke(pm, pb.finish(), 2.0 * s, pal.primary.alpha(a));
            }
        }
    }
}

/// Outlined 8-tooth gear with a center hole, ~18px across at scale 1.
fn gear_icon(cx: f32, cy: f32, s: f32) -> Option<tiny_skia::Path> {
    use std::f32::consts::FRAC_PI_4;
    let (r_in, r_out) = (6.5 * s, 9.0 * s);
    let pt = |r: f32, a: f32| (cx + r * a.cos(), cy + r * a.sin());
    let mut pb = PathBuilder::new();
    for i in 0..8 {
        let a = i as f32 * FRAC_PI_4;
        for (j, (r, da)) in [(r_in, -0.36), (r_out, -0.17), (r_out, 0.17), (r_in, 0.36)].into_iter().enumerate() {
            let (px, py) = pt(r, a + da);
            if i == 0 && j == 0 {
                pb.move_to(px, py);
            } else {
                pb.line_to(px, py);
            }
        }
    }
    pb.close();
    pb.push_circle(cx, cy, 2.8 * s);
    pb.finish()
}

fn box_y(win_h: f32, expand_up: bool, h: f32) -> f32 {
    if expand_up {
        (win_h - h).round().max(0.0)
    } else {
        0.0
    }
}

fn fmt_time(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn fill(pm: &mut Pixmap, path: Option<tiny_skia::Path>, c: Rgba) {
    let Some(path) = path else { return };
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba(c.0, c.1, c.2, c.3.clamp(0.0, 1.0)).unwrap_or(Color::WHITE));
    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn stroke(pm: &mut Pixmap, path: Option<tiny_skia::Path>, width: f32, c: Rgba) {
    let Some(path) = path else { return };
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba(c.0, c.1, c.2, c.3.clamp(0.0, 1.0)).unwrap_or(Color::WHITE));
    let st = Stroke { width, ..Stroke::default() };
    pm.stroke_path(&path, &paint, &st, Transform::identity(), None);
}

fn draw_art(pm: &mut Pixmap, art: Option<&Pixmap>, rect: (f32, f32, f32), r: f32, a: f32, light: bool) {
    let (x, y, size) = rect;
    let Some(path) = rrect(x, y, size, size, r) else { return };
    let mut paint = Paint::default();
    match art {
        Some(img) => {
            let sx = size / img.width().max(1) as f32;
            let sy = size / img.height().max(1) as f32;
            paint.shader = Pattern::new(
                img.as_ref(),
                SpreadMode::Pad,
                FilterQuality::Bilinear,
                a,
                Transform::from_scale(sx, sy).post_translate(x, y),
            );
        }
        None => {
            let v = if light { 0xE2 } else { 0x28 };
            paint.set_color_rgba8(v, v, v, (255.0 * a) as u8);
        }
    }
    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn draw_bars(pm: &mut Pixmap, bars: [f32; 4], pos: (f32, f32), max_h: f32, s: f32, color: Rgba) {
    let (x, cy) = pos;
    for (i, v) in bars.into_iter().enumerate() {
        let h = (3.0 + v * (max_h - 3.0)) * s;
        fill(pm, rrect(x + i as f32 * 5.0 * s, cy - h / 2.0, 3.0 * s, h, 1.5 * s), color);
    }
}

fn tri(pb: &mut PathBuilder, p: [(f32, f32); 3]) {
    pb.move_to(p[0].0, p[0].1);
    pb.line_to(p[1].0, p[1].1);
    pb.line_to(p[2].0, p[2].1);
    pb.close();
}

/// Skip-back icon when `dir = -1.0`, skip-forward when `dir = 1.0`.
fn skip_icon(cx: f32, cy: f32, s: f32, dir: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    tri(&mut pb, [(cx - 5.0 * s * dir, cy - 6.0 * s), (cx - 5.0 * s * dir, cy + 6.0 * s), (cx + 5.0 * s * dir, cy)]);
    let bx = cx + 5.0 * s * dir;
    pb.push_rect(tiny_skia::Rect::from_ltrb(
        bx.min(bx + 2.0 * s * dir),
        cy - 6.0 * s,
        bx.max(bx + 2.0 * s * dir),
        cy + 6.0 * s,
    )?);
    pb.finish()
}

fn play_icon(cx: f32, cy: f32, s: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    tri(&mut pb, [(cx - 5.0 * s, cy - 7.0 * s), (cx - 5.0 * s, cy + 7.0 * s), (cx + 7.0 * s, cy)]);
    pb.finish()
}

fn pause_icon(cx: f32, cy: f32, s: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    for x0 in [cx - 6.0 * s, cx + 2.0 * s] {
        pb.push_rect(tiny_skia::Rect::from_xywh(x0, cy - 7.0 * s, 4.0 * s, 14.0 * s)?);
    }
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel_lum(rgb: [u8; 3]) -> f64 {
        let ch = |v: u8| {
            let c = v as f64 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(rgb[0]) + 0.7152 * ch(rgb[1]) + 0.0722 * ch(rgb[2])
    }

    fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
        let (la, lb) = (rel_lum(a), rel_lum(b));
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn composite(fg: Rgba, bg: [u8; 3]) -> [u8; 3] {
        let a = fg.3.clamp(0.0, 1.0) as f64;
        let mix = |f: f32, b: u8| (f as f64 * 255.0 * a + b as f64 * (1.0 - a)).round() as u8;
        [mix(fg.0, bg[0]), mix(fg.1, bg[1]), mix(fg.2, bg[2])]
    }

    #[test]
    fn pill_height_and_compact_box_stay_stationary_across_expand_collapse() {
        assert_eq!(pill_height(8.0), PILL_H);
        assert_eq!(pill_height(11.0), 34.0);
        assert_eq!(pill_height(18.0), 46.0);

        // Verify compact art never overflows the semicircular pill cap (pad_l + r > pill_r).
        let r_pill = pill_height(11.0) / 2.0;
        assert!(COMPACT_PAD_L + 6.0 > r_pill);

        let now = Instant::now();
        let mut isl = Island::new(now);
        let win = (850, 310);
        let before_down = isl.compact_rect(win, 1.0, 425.0, false);
        let before_up = isl.compact_rect(win, 1.0, 425.0, true);
        isl.expand(now);
        assert_eq!(isl.compact_rect(win, 1.0, 425.0, false), before_down);
        assert_eq!(isl.compact_rect(win, 1.0, 425.0, true), before_up);
        isl.collapse(now + Duration::from_millis(200));
        assert_eq!(isl.compact_rect(win, 1.0, 425.0, false), before_down);
        assert_eq!(isl.compact_rect(win, 1.0, 425.0, true), before_up);
    }

    #[test]
    fn tone_art_gradient_guarantees_wcag_aa_contrast() {
        let covers = [
            [0, 0, 0],
            [255, 255, 255],
            [255, 240, 0],
            [0, 255, 255],
            [220, 20, 60],
        ];
        for light in [false, true] {
            let pal = Palette::for_theme(light);
            let default_bg = if light { [255, 255, 255] } else { [14, 14, 16] };
            let mut bgs = vec![default_bg];
            for c in covers {
                let (ta, tb) = tone_art_gradient(c, c, light);
                assert_eq!(ta, tb);
                if light {
                    assert!(ta.into_iter().all(|v| v >= 0xC7));
                } else {
                    assert!(ta.into_iter().all(|v| v <= 0x47));
                }
                bgs.push(ta);
            }
            for bg in bgs {
                let pri = composite(pal.primary, bg);
                let sec = composite(pal.secondary, bg);
                let kara = composite(pal.karaoke_base, bg);
                let track = composite(pal.track_bg, bg);
                let ring = composite(pal.focus_ring, bg);

                assert!(contrast_ratio(pri, bg) >= 9.0, "primary contrast failed on {bg:?} (light={light})");
                assert!(contrast_ratio(sec, bg) >= 5.5, "secondary contrast failed on {bg:?} (light={light})");
                assert!(contrast_ratio(kara, bg) >= 4.5, "karaoke base contrast failed on {bg:?} (light={light})");
                assert!(contrast_ratio(track, bg) >= 3.0, "progress track contrast failed on {bg:?} (light={light})");
                assert!(contrast_ratio(ring, bg) >= 3.0, "focus ring contrast failed on {bg:?} (light={light})");
            }
        }
    }

    #[test]
    fn hit_at_respects_expanded_state_and_44px_targets() {
        let now = Instant::now();
        let mut isl = Island::new(now);
        isl.prev.set("Earlier line");
        isl.next.set("Upcoming line");

        let mid = EXPANDED_W / 2.0;
        assert_eq!(isl.hit_at(mid, BUTTON_Y, true, true, true), None);
        assert_eq!(isl.hit_at(mid, 193.0, true, true, true), None);

        isl.expand(now);
        assert_eq!(isl.hit_at(mid - 64.0, BUTTON_Y, true, true, true), Some(Hit::Button(Command::Previous)));
        assert_eq!(isl.hit_at(mid, BUTTON_Y, true, true, true), Some(Hit::Button(Command::PlayPause)));
        assert_eq!(isl.hit_at(mid + 64.0, BUTTON_Y, true, true, true), Some(Hit::Button(Command::Next)));
        assert_eq!(isl.hit_at(mid + 21.5, BUTTON_Y, true, true, true), Some(Hit::Button(Command::PlayPause)));
        assert_eq!(isl.hit_at(mid - 32.0, BUTTON_Y, true, true, true), None);

        assert_eq!(isl.hit_at(mid, 193.0, true, true, true), Some(Hit::Progress));
        assert_eq!(isl.hit_at(mid, 193.0, true, true, false), None);

        assert_eq!(isl.hit_at(mid, 92.0, true, true, true), Some(Hit::PrevLine));
        assert_eq!(isl.hit_at(mid, 92.0, false, true, true), None);
        assert_eq!(isl.hit_at(mid, 164.0, true, true, true), Some(Hit::NextLine));
        assert_eq!(isl.hit_at(mid, 164.0, true, false, true), None);
    }

    #[test]
    fn progress_frac_clamps_and_maps_track_bounds() {
        let isl = Island::new(Instant::now());
        assert_eq!(isl.progress_frac(0.0), 0.0);
        assert_eq!(isl.progress_frac(62.0), 0.0);
        assert!((isl.progress_frac(EXPANDED_W / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(isl.progress_frac(EXPANDED_W - 62.0), 1.0);
        assert_eq!(isl.progress_frac(EXPANDED_W), 1.0);
    }

    #[test]
    fn move_focus_cycles_forward_backward_and_clears_on_collapse() {
        let now = Instant::now();
        let mut isl = Island::new(now);
        isl.expand(now);

        let all = [
            Hit::PrevLine,
            Hit::NextLine,
            Hit::Progress,
            Hit::Button(Command::Previous),
            Hit::Button(Command::PlayPause),
            Hit::Button(Command::Next),
            Hit::Settings,
        ];
        for expected in all {
            isl.move_focus(false, true, true, true);
            assert_eq!(isl.focus, Some(expected));
        }
        isl.move_focus(false, true, true, true);
        assert_eq!(isl.focus, Some(Hit::PrevLine));

        isl.move_focus(true, true, true, true);
        assert_eq!(isl.focus, Some(Hit::Settings));

        isl.collapse(now);
        assert_eq!(isl.focus, None);
    }

    #[test]
    fn expanded_shape_grows_away_from_the_aligned_pill_edge_and_menu_rows_hit() {
        let now = Instant::now();
        let mut isl = Island::new(now);
        let win = (850, 310);
        let (cx, pw) = (425.0, isl.pill_w);
        for (align, expect_x) in [(0.0, cx - pw / 2.0), (0.5, cx - EXPANDED_W / 2.0), (1.0, cx + pw / 2.0 - EXPANDED_W)] {
            isl.align = align;
            assert_eq!(isl.expanded_origin(win, 1.0, cx, false).0, expect_x.round(), "align {align}");
            // The compact pill itself never moves with alignment.
            assert_eq!(isl.compact_rect(win, 1.0, cx, false).0, (cx - pw / 2.0).round());
        }

        isl.open_menu(now);
        let first = MENU_PAD + MENU_ROW_H / 2.0;
        assert_eq!(isl.hit_at(MENU_W / 2.0, first, true, true, true), Some(Hit::Menu(MenuItem::Hide)));
        let last = MENU_PAD + MENU_ROW_H * (MENU.len() as f32 - 0.5);
        assert_eq!(isl.hit_at(MENU_W / 2.0, last, true, true, true), Some(Hit::Menu(MenuItem::Quit)));
        assert_eq!(isl.hit_at(MENU_W / 2.0, MENU_H - 2.0, true, true, true), None);
        assert_eq!(isl.hit_at(MENU_W + 4.0, first, true, true, true), None);
        isl.expand(now);
        assert_eq!(isl.hit_at(GEAR.0, GEAR.1, true, true, true), Some(Hit::Settings));
    }
}
