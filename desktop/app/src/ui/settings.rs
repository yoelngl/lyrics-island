//! Companion settings panel for Lyrics Island.
//!
//! Design read: desktop configuration companion for music listeners, monochrome Light/Dark
//! palette aligned with the island pill, dial ENERGY 1 / RHYTHM 2 / MOTION 2.
//! Purpose notes (R-31):
//! - Monochrome surfaces (#0E0E10 dark, #FFFFFF light) keep focus on active lyrics overlays
//!   while meeting WCAG AA contrast (>= 4.5:1 text, >= 3.6:1 control boundaries).
//! - Single accent (#60A5FA dark with #090D16 ink; #1D4ED8 light with #FFFFFF ink) is reserved
//!   strictly for active selection, switch-on state, caret, and the 2px keyboard focus ring.
//! - Section rhythm varies by task: segmented theme bar, grouped preference rows, wide offset
//!   stepping bar, dynamic folder list + path form, and inline shortcut capture rows.
//! - Responsive reflow stacks labels above full-width controls below 460 logical px so narrow
//!   windows never clip or collide.

use std::path::{Path, PathBuf};

use tiny_skia::{
    Color, FillRule, LineCap, Mask, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform,
};
use winit::event::{ElementState, KeyEvent};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

use super::rrect;
use super::text::{style, Clip, Label, Rgba, Style, Text};
use crate::config::{Config, Hotkeys};
use crate::hotkeys::format_shortcut;
use crate::model::{PlayerChoice, Theme, UiAction};

pub const DEFAULT_W: f32 = 520.0;
pub const DEFAULT_H: f32 = 640.0;
pub const MIN_W: f32 = 360.0;
pub const MIN_H: f32 = 460.0;

const HEADER_H: f32 = 56.0;
const NARROW_W: f32 = 460.0;
const SCROLL_MARGIN: f32 = 18.0;

const HEADER_TITLE: Style = style(15.0, 20.0, 700);
const HEADER_SUB: Style = style(12.0, 16.0, 400);
const SECTION_HEAD: Style = style(13.5, 18.0, 700);
const ROW_TITLE: Style = style(13.5, 18.0, 500);
const ROW_SUB: Style = style(12.0, 16.5, 400);
const CTRL_TEXT: Style = style(12.5, 17.0, 600);
const INPUT_TEXT: Style = style(12.5, 17.0, 400);
const VALUE_TEXT: Style = style(13.0, 18.0, 600);

const HOTKEY_LABELS: [&str; 7] = [
    "Show or hide island",
    "Show or hide desktop lyrics",
    "Toggle click-through",
    "Play or pause",
    "Lyrics 250 ms earlier",
    "Lyrics 250 ms later",
    "Reset lyrics offset",
];

const LRC_EMPTY_STATE: &str = "No extra folders configured. Local .lrc files next to audio files load automatically; add a folder below to search Artist - Title.lrc.";
const CAPTURE_PROMPT: &str = "Press keys... (Esc cancel, Backspace disable)";
const POS_RESET_STATUS: &str = "Status: Window positions reset to defaults";

fn lrc_placeholder() -> &'static str {
    if cfg!(windows) {
        "C:\\Music\\Lyrics"
    } else {
        "/path/to/lyrics"
    }
}

#[derive(Clone, Copy)]
struct Palette {
    bg: Color,
    bg_rgba: Rgba,
    elevated_rgba: Rgba,
    hover_rgba: Rgba,
    divider: Rgba,
    ctrl_border: Rgba,
    switch_off_bg: Rgba,
    switch_off_knob: Rgba,
    text_primary: Rgba,
    text_secondary: Rgba,
    accent: Rgba,
    on_accent: Rgba,
    error: Rgba,
    success: Rgba,
    scrollbar: Rgba,
}

impl Palette {
    fn dark() -> Self {
        Self {
            bg: Color::from_rgba8(0x0E, 0x0E, 0x10, 0xFF),
            bg_rgba: Rgba::rgba8(0x0E, 0x0E, 0x10, 1.0),
            elevated_rgba: Rgba::rgba8(0x18, 0x18, 0x1C, 1.0),
            hover_rgba: Rgba::rgba8(0x23, 0x23, 0x29, 1.0),
            divider: Rgba::rgba8(0x34, 0x34, 0x3D, 1.0),
            // #71717A achieves 3.99:1 on #0E0E10 and 3.66:1 on #18181C (WCAG 1.4.11 >= 3:1).
            ctrl_border: Rgba::rgba8(0x71, 0x71, 0x7A, 1.0),
            switch_off_bg: Rgba::rgba8(0x27, 0x27, 0x2A, 1.0),
            switch_off_knob: Rgba::rgba8(0xE4, 0xE4, 0xE7, 1.0),
            text_primary: Rgba::rgba8(0xFF, 0xFF, 0xFF, 1.0),
            text_secondary: Rgba::rgba8(0xA1, 0xA1, 0xAA, 1.0),
            accent: Rgba::rgba8(0x60, 0xA5, 0xFA, 1.0),
            // Dark ink on #60A5FA achieves 7.64:1 contrast.
            on_accent: Rgba::rgba8(0x09, 0x0D, 0x16, 1.0),
            error: Rgba::rgba8(0xFC, 0xA5, 0xA5, 1.0),
            success: Rgba::rgba8(0x6E, 0xE7, 0xB7, 1.0),
            scrollbar: Rgba::rgba8(0x71, 0x71, 0x7A, 0.8),
        }
    }

    fn light() -> Self {
        Self {
            bg: Color::from_rgba8(0xFF, 0xFF, 0xFF, 0xFF),
            bg_rgba: Rgba::rgba8(0xFF, 0xFF, 0xFF, 1.0),
            elevated_rgba: Rgba::rgba8(0xF4, 0xF4, 0xF5, 1.0),
            hover_rgba: Rgba::rgba8(0xE4, 0xE4, 0xE7, 1.0),
            divider: Rgba::rgba8(0xC4, 0xC4, 0xCC, 1.0),
            // #71717A achieves 4.83:1 on #FFFFFF and 4.40:1 on #F4F4F5.
            ctrl_border: Rgba::rgba8(0x71, 0x71, 0x7A, 1.0),
            switch_off_bg: Rgba::rgba8(0x71, 0x71, 0x7A, 1.0),
            switch_off_knob: Rgba::rgba8(0xFF, 0xFF, 0xFF, 1.0),
            text_primary: Rgba::rgba8(0x11, 0x11, 0x13, 1.0),
            text_secondary: Rgba::rgba8(0x52, 0x52, 0x5B, 1.0),
            accent: Rgba::rgba8(0x1D, 0x4E, 0xD8, 1.0),
            on_accent: Rgba::rgba8(0xFF, 0xFF, 0xFF, 1.0),
            error: Rgba::rgba8(0xB9, 0x1C, 0x1C, 1.0),
            success: Rgba::rgba8(0x15, 0x80, 0x3D, 1.0),
            scrollbar: Rgba::rgba8(0x71, 0x71, 0x7A, 0.75),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RectL {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl RectL {
    const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    fn inflate(&self, d: f32) -> Self {
        Self { x: self.x - d, y: self.y - d, w: self.w + 2.0 * d, h: self.h + 2.0 * d }
    }

    fn scaled(&self, scale: f32, dy: f32) -> Self {
        Self {
            x: self.x * scale,
            y: (self.y + dy) * scale,
            w: self.w * scale,
            h: self.h * scale,
        }
    }

    #[cfg(test)]
    fn intersects(&self, other: &Self) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControlId {
    Close,
    ThemeSystem,
    ThemeDark,
    ThemeLight,
    ToggleIsland,
    ToggleDesktop,
    ToggleKaraoke,
    ToggleClickThrough,
    MaxWidthDec,
    MaxWidthInc,
    CompactFontDec,
    CompactFontInc,
    DesktopFontDec,
    DesktopFontInc,
    ResetPositions,
    OffsetReset,
    OffsetM250,
    OffsetM50,
    OffsetP50,
    OffsetP250,
    MediaSource,
    ToggleAutostart,
    RemoveLrcDir(usize),
    LrcInput,
    AddLrcDir,
    Hotkey(usize),
    RestoreHotkeys,
}

#[derive(Clone, Copy, Debug)]
struct ControlBox {
    id: ControlId,
    in_header: bool,
    vis: RectL,
    hit: RectL,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Feedback {
    Status(String),
    Error(String),
}

struct LabelPool {
    items: Vec<Label>,
    used: usize,
}

impl LabelPool {
    fn new() -> Self {
        Self { items: Vec::new(), used: 0 }
    }

    fn begin(&mut self) {
        self.used = 0;
    }

    fn next(&mut self, text: &str) -> &mut Label {
        if self.used == self.items.len() {
            self.items.push(Label::new());
        }
        let lbl = &mut self.items[self.used];
        self.used += 1;
        lbl.set(text);
        lbl
    }
}

/// Deterministic geometry for both hit-testing and rendering at a given logical window width.
struct Layout {
    narrow: bool,
    pad_x: f32,
    card_w: f32,
    inner_x: f32,
    inner_w: f32,
    content_h: f32,
    controls: Vec<ControlBox>,

    sec_appearance_y: f32,
    theme_card: RectL,
    theme_label_y: f32,

    app_card: RectL,
    row_island_y: f32,
    row_desktop_y: f32,
    row_karaoke_y: f32,
    row_click_y: f32,
    row_click_h: f32,
    row_max_w_y: f32,
    max_w_val: RectL,
    row_cfont_y: f32,
    cfont_val: RectL,
    row_dfont_y: f32,
    dfont_val: RectL,
    row_pos_y: f32,
    row_pos_h: f32,

    sec_timing_y: f32,
    timing_card: RectL,
    row_offset_y: f32,
    row_offset_h: f32,
    offset_readout_pos: (f32, f32),
    row_media_y: f32,
    row_media_h: f32,
    row_autostart_y: f32,
    row_autostart_h: f32,

    sec_lrc_y: f32,
    lrc_card: RectL,
    lrc_empty_rect: Option<RectL>,
    lrc_dir_rows: Vec<(f32, f32)>,
    lrc_form_y: f32,
    lrc_feedback_y: Option<f32>,

    sec_hotkeys_y: f32,
    hotkeys_card: RectL,
    hotkey_rows: [(f32, f32); 7],
    restore_row_y: f32,
}

impl Layout {
    fn max_scroll(&self, win_h: f32) -> f32 {
        let body_h = (win_h - HEADER_H).max(1.0);
        (self.content_h - body_h).max(0.0)
    }

    fn find(&self, id: ControlId) -> Option<&ControlBox> {
        self.controls.iter().find(|c| c.id == id)
    }
}

pub struct Settings {
    cfg: Config,
    autostart: bool,
    players: Vec<PlayerChoice>,
    status_line: String,
    hotkey_errors: Vec<(usize, String)>,
    autostart_error: Option<String>,
    recording_hotkey: Option<usize>,

    scroll_y: f32,
    last_win_w: f32,
    hover: Option<ControlId>,
    focus: Option<ControlId>,

    folder_input: String,
    lrc_feedback: Option<Feedback>,
    positions_status: Option<String>,
    media_status: Option<String>,
    hotkey_status: Option<String>,

    labels: LabelPool,
}

impl Settings {
    pub fn new(cfg: &crate::config::Config, autostart: bool) -> Self {
        Self {
            cfg: cfg.clone(),
            autostart,
            players: Vec::new(),
            status_line: "No active media player".into(),
            hotkey_errors: Vec::new(),
            autostart_error: None,
            recording_hotkey: None,
            scroll_y: 0.0,
            last_win_w: DEFAULT_W,
            hover: None,
            focus: None,
            folder_input: String::new(),
            lrc_feedback: None,
            positions_status: None,
            media_status: None,
            hotkey_status: None,
            labels: LabelPool::new(),
        }
    }

    pub fn sync(&mut self, cfg: &crate::config::Config, autostart: bool) {
        self.cfg = cfg.clone();
        self.autostart = autostart;
    }

    pub fn set_players(&mut self, players: Vec<PlayerChoice>) {
        self.players = players;
        if !self.players.is_empty() {
            self.media_status = None;
        }
    }

    pub fn set_status_line(&mut self, summary: &str) {
        let trimmed = summary.trim();
        self.status_line.clear();
        if trimmed.is_empty() {
            self.status_line.push_str("No active media player");
        } else {
            self.status_line.push_str(trimmed);
        }
    }

    pub fn set_hotkey_errors(&mut self, errs: Vec<(usize, String)>) {
        self.hotkey_errors = errs;
    }

    pub fn set_autostart_error(&mut self, err: Option<String>) {
        self.autostart_error = err;
    }

    pub fn is_recording_hotkey(&self) -> bool {
        self.recording_hotkey.is_some()
    }

    /// Logical px coordinates. True when pressing in the top header bar outside the Close button.
    pub fn is_drag_header(&self, x: f32, y: f32, win_w: f32) -> bool {
        let w = sanitize_w(win_w);
        (0.0..w).contains(&x) && (0.0..HEADER_H).contains(&y) && !close_hit_rect(w).contains(x, y)
    }

    pub fn on_cursor_move(&mut self, x: f32, y: f32, win_w: f32, win_h: f32) -> bool {
        if win_w.is_finite() && win_w > 0.0 {
            self.last_win_w = win_w;
        }
        let layout = self.compute_layout(self.last_win_w);
        self.scroll_y = self.scroll_y.clamp(0.0, layout.max_scroll(win_h));
        let next_hover = self.hit_test(&layout, x, y, self.last_win_w, win_h);
        if self.hover != next_hover {
            self.hover = next_hover;
            true
        } else {
            false
        }
    }

    pub fn on_mouse_wheel(&mut self, dy_logical: f32, win_h: f32) -> bool {
        if !dy_logical.is_finite() {
            return false;
        }
        let layout = self.compute_layout(self.last_win_w);
        let max_scroll = layout.max_scroll(win_h);
        let next = (self.scroll_y - dy_logical).clamp(0.0, max_scroll);
        if (next - self.scroll_y).abs() > 0.01 {
            self.scroll_y = next;
            true
        } else {
            false
        }
    }

    /// Returns `(actions, close_requested, redraw_needed)`.
    pub fn on_mouse_down(
        &mut self,
        x: f32,
        y: f32,
        win_w: f32,
        win_h: f32,
    ) -> (Vec<UiAction>, bool, bool) {
        if win_w.is_finite() && win_w > 0.0 {
            self.last_win_w = win_w;
        }
        let layout = self.compute_layout(self.last_win_w);
        self.scroll_y = self.scroll_y.clamp(0.0, layout.max_scroll(win_h));

        let hit = self.hit_test(&layout, x, y, self.last_win_w, win_h);
        self.hover = hit;

        // Clicking anywhere outside the active shortcut capture cancels recording cleanly.
        if let Some(rec_idx) = self.recording_hotkey {
            if hit != Some(ControlId::Hotkey(rec_idx)) {
                self.recording_hotkey = None;
                if hit.is_none() {
                    return (Vec::new(), false, true);
                }
            }
        }

        let Some(id) = hit else {
            let had_input_focus = self.focus == Some(ControlId::LrcInput);
            if had_input_focus {
                self.focus = None;
            }
            return (Vec::new(), false, had_input_focus);
        };

        self.focus = Some(id);
        self.activate_control(id, win_h)
    }

    /// Returns `(actions, close_requested, redraw_needed)`.
    pub fn on_key(
        &mut self,
        event: &KeyEvent,
        mods: ModifiersState,
        win_h: f32,
    ) -> (Vec<UiAction>, bool, bool) {
        self.handle_key(
            event.state,
            &event.logical_key,
            event.physical_key,
            event.text.as_deref(),
            mods,
            win_h,
        )
    }

    pub fn render(&mut self, pm: &mut Pixmap, t: &mut Text, scale: f32, light: bool) {
        if pm.width() == 0 || pm.height() == 0 {
            return;
        }
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let win_w = pm.width() as f32 / scale;
        let win_h = pm.height() as f32 / scale;
        self.last_win_w = win_w;

        let pal = if light { Palette::light() } else { Palette::dark() };
        pm.fill(pal.bg);

        let layout = self.compute_layout(win_w);
        let max_scroll = layout.max_scroll(win_h);
        self.scroll_y = self.scroll_y.clamp(0.0, max_scroll);

        let pw = pm.width() as f32;
        let ph = pm.height() as f32;
        let header_h_px = (HEADER_H * scale).round();
        let body_clip: Clip = (0.0, header_h_px, pw, ph);
        let dy = HEADER_H - self.scroll_y;

        let mut body_mask = Mask::new(pm.width(), pm.height());
        if let Some(m) = body_mask.as_mut() {
            if let Some(rect) = Rect::from_ltrb(0.0, header_h_px, pw, ph) {
                let mut pb = PathBuilder::new();
                pb.push_rect(rect);
                if let Some(path) = pb.finish() {
                    m.fill_path(&path, FillRule::Winding, false, Transform::identity());
                }
            }
        }
        let mask_ref = body_mask.as_ref();

        self.labels.begin();

        self.render_appearance(pm, t, &pal, &layout, scale, dy, body_clip, mask_ref);
        self.render_timing(pm, t, &pal, &layout, scale, dy, body_clip, mask_ref);
        self.render_lrc_dirs(pm, t, &pal, &layout, scale, dy, body_clip, mask_ref);
        self.render_hotkeys(pm, t, &pal, &layout, scale, dy, body_clip, mask_ref);

        // Scrollbar thumb on the right edge when content exceeds the body viewport.
        let body_h = (win_h - HEADER_H).max(1.0);
        if layout.content_h > body_h + 0.5 {
            let track_h = (body_h - 12.0).max(20.0);
            let thumb_h = ((body_h / layout.content_h) * track_h).clamp(28.0, track_h);
            let frac = if max_scroll > 0.0 { (self.scroll_y / max_scroll).clamp(0.0, 1.0) } else { 0.0 };
            let thumb_y = HEADER_H + 6.0 + frac * (track_h - thumb_h);
            let thumb_x = win_w - if layout.narrow { 6.5 } else { 8.5 };
            fill_rrect(
                pm,
                RectL::new(thumb_x, thumb_y, 4.0, thumb_h).scaled(scale, 0.0),
                2.0 * scale,
                pal.scrollbar,
                mask_ref,
            );
        }

        // Sticky header rendered last so scrolled controls never bleed over it.
        self.render_header(pm, t, &pal, &layout, scale, win_w, header_h_px);
    }

    fn handle_key(
        &mut self,
        state: ElementState,
        logical_key: &Key,
        physical_key: PhysicalKey,
        text: Option<&str>,
        mods: ModifiersState,
        win_h: f32,
    ) -> (Vec<UiAction>, bool, bool) {
        if state != ElementState::Pressed {
            return (Vec::new(), false, false);
        }

        // Priority 1: active global hotkey capture.
        if let Some(idx) = self.recording_hotkey {
            if *logical_key == Key::Named(NamedKey::Escape) {
                self.recording_hotkey = None;
                self.hotkey_status = Some("Status: Shortcut capture cancelled".into());
                return (Vec::new(), false, true);
            }
            if matches!(logical_key, Key::Named(NamedKey::Backspace | NamedKey::Delete)) {
                if let Some(field) = self.cfg.hotkeys.field_mut(idx) {
                    field.clear();
                }
                self.recording_hotkey = None;
                self.hotkey_errors.retain(|(i, _)| *i != idx);
                self.hotkey_status = Some(format!("Status: Disabled '{}'", HOTKEY_LABELS[idx]));
                return (vec![UiAction::SetHotkeys(self.cfg.hotkeys.clone())], false, true);
            }
            if let PhysicalKey::Code(code) = physical_key {
                if let Some(accel) = format_shortcut(mods, code) {
                    if let Some(field) = self.cfg.hotkeys.field_mut(idx) {
                        *field = accel.clone();
                    }
                    self.recording_hotkey = None;
                    self.hotkey_errors.retain(|(i, _)| *i != idx);
                    self.hotkey_status =
                        Some(format!("Status: Bound '{}' to {accel}", HOTKEY_LABELS[idx]));
                    return (vec![UiAction::SetHotkeys(self.cfg.hotkeys.clone())], false, true);
                }
            }
            return (Vec::new(), false, false);
        }

        // Priority 2: Escape clears folder input focus, or closes the window when idle.
        if *logical_key == Key::Named(NamedKey::Escape) {
            if self.focus == Some(ControlId::LrcInput) {
                self.focus = None;
                return (Vec::new(), false, true);
            }
            return (Vec::new(), true, false);
        }

        // Priority 3: sequential Tab / Shift+Tab focus navigation in strict visual order.
        if *logical_key == Key::Named(NamedKey::Tab) {
            let layout = self.compute_layout(self.last_win_w);
            if layout.controls.is_empty() {
                return (Vec::new(), false, false);
            }
            let len = layout.controls.len();
            let cur_idx = self.focus.and_then(|f| layout.controls.iter().position(|c| c.id == f));
            let next_idx = if mods.shift_key() {
                match cur_idx {
                    Some(0) | None => len - 1,
                    Some(i) => i - 1,
                }
            } else {
                match cur_idx {
                    Some(i) if i + 1 < len => i + 1,
                    _ => 0,
                }
            };
            self.focus = Some(layout.controls[next_idx].id);
            self.ensure_focused_visible(&layout, win_h);
            return (Vec::new(), false, true);
        }

        // PageUp / PageDown scrolling works from any focus state.
        if matches!(logical_key, Key::Named(NamedKey::PageUp | NamedKey::PageDown)) {
            let body_h = (win_h - HEADER_H).max(80.0);
            let step = (body_h * 0.85).max(80.0);
            let dy = if *logical_key == Key::Named(NamedKey::PageUp) { step } else { -step };
            let moved = self.on_mouse_wheel(dy, win_h);
            return (Vec::new(), false, moved);
        }

        // Priority 4: typing, pasting, Backspace, and Enter inside the .lrc folder input.
        if self.focus == Some(ControlId::LrcInput) {
            if *logical_key == Key::Named(NamedKey::Enter) {
                return self.submit_lrc_folder(win_h);
            }
            if *logical_key == Key::Named(NamedKey::Backspace) {
                if mods.control_key() {
                    self.folder_input.clear();
                } else {
                    self.folder_input.pop();
                }
                if matches!(self.lrc_feedback, Some(Feedback::Error(_))) {
                    self.lrc_feedback = None;
                }
                return (Vec::new(), false, true);
            }
            if *logical_key == Key::Named(NamedKey::Delete) {
                self.folder_input.clear();
                if matches!(self.lrc_feedback, Some(Feedback::Error(_))) {
                    self.lrc_feedback = None;
                }
                return (Vec::new(), false, true);
            }
            let is_paste_v = (mods.control_key() || mods.super_key())
                && (physical_key == PhysicalKey::Code(KeyCode::KeyV)
                    || matches!(logical_key, Key::Character(s) if s.eq_ignore_ascii_case("v")));
            if is_paste_v {
                if let Some(clip_str) = read_clipboard_text() {
                    self.folder_input.push_str(&clip_str);
                    if matches!(self.lrc_feedback, Some(Feedback::Error(_))) {
                        self.lrc_feedback = None;
                    }
                    return (Vec::new(), false, true);
                }
                return (Vec::new(), false, false);
            }
            if !mods.control_key() && !mods.super_key() {
                let mut appended = false;
                if let Some(s) = text {
                    for ch in s.chars().filter(|c| !c.is_control()) {
                        self.folder_input.push(ch);
                        appended = true;
                    }
                } else if *logical_key == Key::Named(NamedKey::Space) {
                    self.folder_input.push(' ');
                    appended = true;
                } else if let Key::Character(s) = logical_key {
                    for ch in s.chars().filter(|c| !c.is_control()) {
                        self.folder_input.push(ch);
                        appended = true;
                    }
                }
                if appended {
                    if matches!(self.lrc_feedback, Some(Feedback::Error(_))) {
                        self.lrc_feedback = None;
                    }
                    return (Vec::new(), false, true);
                }
            }
            return (Vec::new(), false, false);
        }

        // Home / End scroll when not editing text.
        if matches!(logical_key, Key::Named(NamedKey::Home | NamedKey::End)) {
            let layout = self.compute_layout(self.last_win_w);
            let target = if *logical_key == Key::Named(NamedKey::Home) {
                0.0
            } else {
                layout.max_scroll(win_h)
            };
            if (target - self.scroll_y).abs() > 0.01 {
                self.scroll_y = target;
                return (Vec::new(), false, true);
            }
            return (Vec::new(), false, false);
        }

        // ArrowUp / ArrowDown gently scroll the panel when not editing text.
        if matches!(logical_key, Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown)) {
            let dy = if *logical_key == Key::Named(NamedKey::ArrowUp) { 40.0 } else { -40.0 };
            let moved = self.on_mouse_wheel(dy, win_h);
            return (Vec::new(), false, moved);
        }

        let Some(focused) = self.focus else {
            return (Vec::new(), false, false);
        };

        // ArrowLeft / ArrowRight adjust segmented theme selector, steppers, and media source.
        if matches!(logical_key, Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight)) {
            let dir = if *logical_key == Key::Named(NamedKey::ArrowRight) { 1 } else { -1 };
            return self.adjust_focused_horizontal(focused, dir, mods.shift_key());
        }

        let is_activate = *logical_key == Key::Named(NamedKey::Enter)
            || *logical_key == Key::Named(NamedKey::Space)
            || matches!(logical_key, Key::Character(s) if s == " ");
        if is_activate {
            return self.activate_control(focused, win_h);
        }

        (Vec::new(), false, false)
    }

    fn adjust_focused_horizontal(
        &mut self,
        focused: ControlId,
        dir: i32,
        fine_boost: bool,
    ) -> (Vec<UiAction>, bool, bool) {
        match focused {
            ControlId::ThemeSystem | ControlId::ThemeDark | ControlId::ThemeLight => {
                let themes = [
                    (Theme::System, ControlId::ThemeSystem),
                    (Theme::Dark, ControlId::ThemeDark),
                    (Theme::Light, ControlId::ThemeLight),
                ];
                let cur = themes.iter().position(|(t, _)| *t == self.cfg.ui.theme).unwrap_or(0);
                let next = (cur as i32 + dir).rem_euclid(3) as usize;
                self.cfg.ui.theme = themes[next].0;
                self.focus = Some(themes[next].1);
                (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
            }
            ControlId::MaxWidthDec | ControlId::MaxWidthInc => {
                self.step_max_width(25.0 * dir as f32)
            }
            ControlId::CompactFontDec | ControlId::CompactFontInc => {
                self.step_compact_font(1.0 * dir as f32)
            }
            ControlId::DesktopFontDec | ControlId::DesktopFontInc => {
                self.step_desktop_font(2.0 * dir as f32)
            }
            ControlId::OffsetReset
            | ControlId::OffsetM250
            | ControlId::OffsetM50
            | ControlId::OffsetP50
            | ControlId::OffsetP250 => {
                let step = if fine_boost { 250 } else { 50 } * dir as i64;
                self.step_offset(step)
            }
            ControlId::MediaSource => self.cycle_media_source(dir),
            _ => (Vec::new(), false, false),
        }
    }

    fn activate_control(&mut self, id: ControlId, win_h: f32) -> (Vec<UiAction>, bool, bool) {
        match id {
            ControlId::Close => (Vec::new(), true, false),
            ControlId::ThemeSystem => self.set_theme(Theme::System),
            ControlId::ThemeDark => self.set_theme(Theme::Dark),
            ControlId::ThemeLight => self.set_theme(Theme::Light),
            ControlId::ToggleIsland => {
                self.cfg.ui.island_visible = !self.cfg.ui.island_visible;
                (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
            }
            ControlId::ToggleDesktop => {
                self.cfg.ui.desktop_visible = !self.cfg.ui.desktop_visible;
                (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
            }
            ControlId::ToggleKaraoke => {
                self.cfg.ui.karaoke = !self.cfg.ui.karaoke;
                (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
            }
            ControlId::ToggleClickThrough => {
                self.cfg.ui.click_through = !self.cfg.ui.click_through;
                (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
            }
            ControlId::MaxWidthDec => self.step_max_width(-25.0),
            ControlId::MaxWidthInc => self.step_max_width(25.0),
            ControlId::CompactFontDec => self.step_compact_font(-1.0),
            ControlId::CompactFontInc => self.step_compact_font(1.0),
            ControlId::DesktopFontDec => self.step_desktop_font(-2.0),
            ControlId::DesktopFontInc => self.step_desktop_font(2.0),
            ControlId::ResetPositions => {
                self.cfg.ui.island_pos = None;
                self.cfg.ui.desktop_pos = None;
                self.positions_status = Some(POS_RESET_STATUS.into());
                let layout = self.compute_layout(self.last_win_w);
                self.ensure_focused_visible(&layout, win_h);
                (vec![UiAction::ResetPositions], false, true)
            }
            ControlId::OffsetReset => {
                if self.cfg.ui.offset_ms != 0 {
                    self.cfg.ui.offset_ms = 0;
                    if self.focus == Some(ControlId::OffsetReset) {
                        self.focus = Some(ControlId::OffsetM250);
                    }
                    (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
                } else {
                    (Vec::new(), false, false)
                }
            }
            ControlId::OffsetM250 => self.step_offset(-250),
            ControlId::OffsetM50 => self.step_offset(-50),
            ControlId::OffsetP50 => self.step_offset(50),
            ControlId::OffsetP250 => self.step_offset(250),
            ControlId::MediaSource => self.cycle_media_source(1),
            ControlId::ToggleAutostart => {
                self.autostart = !self.autostart;
                self.autostart_error = None;
                (vec![UiAction::SetAutostart(self.autostart)], false, true)
            }
            ControlId::RemoveLrcDir(idx) => {
                if idx < self.cfg.lrc_dirs.len() {
                    self.cfg.lrc_dirs.remove(idx);
                    self.lrc_feedback = Some(Feedback::Status("Status: Folder removed".into()));
                    if self.focus == Some(ControlId::RemoveLrcDir(idx)) {
                        self.focus = Some(if self.cfg.lrc_dirs.is_empty() {
                            ControlId::LrcInput
                        } else {
                            ControlId::RemoveLrcDir(idx.min(self.cfg.lrc_dirs.len() - 1))
                        });
                    }
                    let layout = self.compute_layout(self.last_win_w);
                    self.scroll_y = self.scroll_y.clamp(0.0, layout.max_scroll(win_h));
                    (vec![UiAction::SetLrcDirs(self.cfg.lrc_dirs.clone())], false, true)
                } else {
                    (Vec::new(), false, false)
                }
            }
            ControlId::LrcInput => (Vec::new(), false, true),
            ControlId::AddLrcDir => self.submit_lrc_folder(win_h),
            ControlId::Hotkey(idx) => {
                if self.recording_hotkey == Some(idx) {
                    self.recording_hotkey = None;
                } else {
                    self.recording_hotkey = Some(idx);
                    self.hotkey_status = None;
                }
                let layout = self.compute_layout(self.last_win_w);
                self.ensure_focused_visible(&layout, win_h);
                (Vec::new(), false, true)
            }
            ControlId::RestoreHotkeys => {
                self.cfg.hotkeys = Hotkeys::default();
                self.recording_hotkey = None;
                self.hotkey_errors.clear();
                self.hotkey_status = Some("Status: Default shortcuts restored".into());
                let layout = self.compute_layout(self.last_win_w);
                self.ensure_focused_visible(&layout, win_h);
                (vec![UiAction::SetHotkeys(self.cfg.hotkeys.clone())], false, true)
            }
        }
    }

    fn set_theme(&mut self, theme: Theme) -> (Vec<UiAction>, bool, bool) {
        if self.cfg.ui.theme == theme {
            return (Vec::new(), false, true);
        }
        self.cfg.ui.theme = theme;
        (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
    }

    fn step_max_width(&mut self, delta: f32) -> (Vec<UiAction>, bool, bool) {
        let next = (self.cfg.ui.island_max_width + delta).round().clamp(200.0, 1400.0);
        if (next - self.cfg.ui.island_max_width).abs() < 0.1 {
            return (Vec::new(), false, false);
        }
        self.cfg.ui.island_max_width = next;
        (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
    }

    fn step_compact_font(&mut self, delta: f32) -> (Vec<UiAction>, bool, bool) {
        let next = (self.cfg.ui.island_font_pt + delta).round().clamp(8.0, 18.0);
        if (next - self.cfg.ui.island_font_pt).abs() < 0.1 {
            return (Vec::new(), false, false);
        }
        self.cfg.ui.island_font_pt = next;
        (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
    }

    fn step_desktop_font(&mut self, delta: f32) -> (Vec<UiAction>, bool, bool) {
        let next = (self.cfg.ui.desktop_font_px + delta).round().clamp(16.0, 96.0);
        if (next - self.cfg.ui.desktop_font_px).abs() < 0.1 {
            return (Vec::new(), false, false);
        }
        self.cfg.ui.desktop_font_px = next;
        (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
    }

    fn step_offset(&mut self, delta_ms: i64) -> (Vec<UiAction>, bool, bool) {
        let next = (self.cfg.ui.offset_ms + delta_ms).clamp(-30_000, 30_000);
        if next == self.cfg.ui.offset_ms {
            return (Vec::new(), false, false);
        }
        self.cfg.ui.offset_ms = next;
        (vec![UiAction::SetUiConfig(self.cfg.ui.clone())], false, true)
    }

    fn cycle_media_source(&mut self, dir: i32) -> (Vec<UiAction>, bool, bool) {
        let mut options: Vec<Option<String>> = Vec::with_capacity(self.players.len() + 2);
        options.push(None);
        for p in &self.players {
            options.push(Some(p.id.clone()));
        }
        if let Some(cur) = &self.cfg.preferred_player {
            if !options.iter().any(|o| o.as_deref() == Some(cur.as_str())) {
                options.push(Some(cur.clone()));
            }
        }
        if options.len() <= 1 {
            self.cfg.preferred_player = None;
            self.media_status =
                Some("Status: Automatic source active; open a media player to pin it".into());
            return (vec![UiAction::SelectPlayer(None)], false, true);
        }
        let cur_idx = options.iter().position(|o| *o == self.cfg.preferred_player).unwrap_or(0);
        let next_idx = (cur_idx as i32 + dir).rem_euclid(options.len() as i32) as usize;
        let next = options[next_idx].clone();
        self.cfg.preferred_player = next.clone();
        self.media_status = None;
        (vec![UiAction::SelectPlayer(next)], false, true)
    }

    fn submit_lrc_folder(&mut self, win_h: f32) -> (Vec<UiAction>, bool, bool) {
        let trimmed = self.folder_input.trim();
        if trimmed.is_empty() {
            self.lrc_feedback =
                Some(Feedback::Error("Error: folder not found (enter a directory path)".into()));
            let layout = self.compute_layout(self.last_win_w);
            self.ensure_focused_visible(&layout, win_h);
            return (Vec::new(), false, true);
        }
        let candidate = Path::new(trimmed);
        if !candidate.is_dir() {
            self.lrc_feedback =
                Some(Feedback::Error(format!("Error: folder not found: {trimmed}")));
            let layout = self.compute_layout(self.last_win_w);
            self.ensure_focused_visible(&layout, win_h);
            return (Vec::new(), false, true);
        }
        let pb = PathBuf::from(trimmed);
        if self.cfg.lrc_dirs.iter().any(|existing| existing == &pb) {
            self.lrc_feedback =
                Some(Feedback::Error("Error: Folder is already in the list".into()));
            let layout = self.compute_layout(self.last_win_w);
            self.ensure_focused_visible(&layout, win_h);
            return (Vec::new(), false, true);
        }
        self.cfg.lrc_dirs.push(pb);
        self.folder_input.clear();
        self.lrc_feedback = Some(Feedback::Status("Status: Folder added".into()));
        let layout = self.compute_layout(self.last_win_w);
        self.ensure_focused_visible(&layout, win_h);
        (vec![UiAction::SetLrcDirs(self.cfg.lrc_dirs.clone())], false, true)
    }

    fn ensure_focused_visible(&mut self, layout: &Layout, win_h: f32) {
        let Some(focused) = self.focus else { return };
        let Some(ctrl) = layout.find(focused) else { return };
        if ctrl.in_header {
            return;
        }
        let body_h = (win_h - HEADER_H).max(1.0);
        let top = (ctrl.vis.y - SCROLL_MARGIN).max(0.0);
        let bottom = ctrl.vis.y + ctrl.vis.h + SCROLL_MARGIN;
        if top < self.scroll_y {
            self.scroll_y = top;
        } else if bottom > self.scroll_y + body_h {
            self.scroll_y = bottom - body_h;
        }
        self.scroll_y = self.scroll_y.clamp(0.0, layout.max_scroll(win_h));
    }

    fn hit_test(
        &self,
        layout: &Layout,
        x: f32,
        y: f32,
        win_w: f32,
        win_h: f32,
    ) -> Option<ControlId> {
        let w = sanitize_w(win_w);
        if !x.is_finite() || !y.is_finite() || x < 0.0 || x >= w || y < 0.0 || y >= win_h {
            return None;
        }
        if y < HEADER_H {
            return layout
                .controls
                .iter()
                .find(|c| c.in_header && c.hit.contains(x, y))
                .map(|c| c.id);
        }
        let cy = y - HEADER_H + self.scroll_y;
        layout.controls.iter().find(|c| !c.in_header && c.hit.contains(x, cy)).map(|c| c.id)
    }

    fn hotkey_error_for(&self, idx: usize) -> Option<&str> {
        self.hotkey_errors.iter().find(|(i, _)| *i == idx).map(|(_, msg)| msg.as_str())
    }

    fn media_source_label(&self) -> String {
        match &self.cfg.preferred_player {
            None => "Automatic".into(),
            Some(id) => {
                if let Some(p) = self.players.iter().find(|p| p.id == *id) {
                    p.name.clone()
                } else {
                    id.strip_prefix("org.mpris.MediaPlayer2.").unwrap_or(id).to_owned()
                }
            }
        }
    }

    fn click_through_subtitle(&self) -> String {
        let accel = self.cfg.hotkeys.toggle_click_through.trim();
        if accel.is_empty() {
            "Overlays ignore mouse input (shortcut disabled below)".into()
        } else {
            format!("Overlays ignore mouse input; press {accel} to turn off")
        }
    }

    fn compute_layout(&self, win_w: f32) -> Layout {
        let w = sanitize_w(win_w);
        let narrow = w < NARROW_W;
        let pad_x = if narrow { 14.0 } else { 20.0 };
        let card_w = (w - 2.0 * pad_x).max(200.0);
        let inner_x = pad_x + 12.0;
        let inner_w = (card_w - 24.0).max(176.0);

        let mut controls = Vec::with_capacity(28 + self.cfg.lrc_dirs.len());
        controls.push(ControlBox {
            id: ControlId::Close,
            in_header: true,
            vis: RectL::new(w - 46.0, 10.0, 36.0, 36.0),
            hit: close_hit_rect(w),
        });

        let mut y = 16.0;

        // 1. Appearance
        let sec_appearance_y = y;
        y += 26.0;

        let theme_card_y = y;
        let (theme_card_h, theme_label_y, seg_x, seg_y, seg_total_w) = if narrow {
            (82.0, theme_card_y + 10.0, inner_x, theme_card_y + 36.0, inner_w)
        } else {
            let sw = 240.0f32.min(inner_w);
            (52.0, theme_card_y + 17.0, inner_x + inner_w - sw, theme_card_y + 8.0, sw)
        };
        let seg_gap = 6.0;
        let seg_w = ((seg_total_w - 2.0 * seg_gap) / 3.0).max(48.0);
        for (i, id) in
            [ControlId::ThemeSystem, ControlId::ThemeDark, ControlId::ThemeLight].into_iter().enumerate()
        {
            let sx = seg_x + i as f32 * (seg_w + seg_gap);
            controls.push(ControlBox {
                id,
                in_header: false,
                vis: RectL::new(sx, seg_y, seg_w, 36.0),
                hit: RectL::new(sx, seg_y - 4.0, seg_w, 44.0),
            });
        }
        let theme_card = RectL::new(pad_x, theme_card_y, card_w, theme_card_h);
        y += theme_card_h + 10.0;

        let app_card_y = y;
        let toggle_w = 82.0;

        let row_island_y = y;
        push_toggle_control(&mut controls, ControlId::ToggleIsland, narrow, inner_x, inner_w, toggle_w, y);
        y += if narrow { 78.0 } else { 49.0 };

        let row_desktop_y = y;
        push_toggle_control(&mut controls, ControlId::ToggleDesktop, narrow, inner_x, inner_w, toggle_w, y);
        y += if narrow { 78.0 } else { 49.0 };

        let row_karaoke_y = y;
        push_toggle_control(&mut controls, ControlId::ToggleKaraoke, narrow, inner_x, inner_w, toggle_w, y);
        y += if narrow { 78.0 } else { 49.0 };

        let row_click_y = y;
        let row_click_h = if narrow { 112.0 } else { 68.0 };
        {
            let (vx, vy) = if narrow {
                (inner_x, row_click_y + 68.0)
            } else {
                (inner_x + inner_w - toggle_w, row_click_y + (row_click_h - 36.0) / 2.0)
            };
            controls.push(ControlBox {
                id: ControlId::ToggleClickThrough,
                in_header: false,
                vis: RectL::new(vx, vy, toggle_w, 36.0),
                hit: RectL::new(vx, vy - 4.0, toggle_w, 44.0),
            });
        }
        y += row_click_h + 1.0;

        let row_max_w_y = y;
        let max_w_val = push_stepper_controls(
            &mut controls,
            ControlId::MaxWidthDec,
            ControlId::MaxWidthInc,
            narrow,
            inner_x,
            inner_w,
            y,
        );
        y += if narrow { 78.0 } else { 49.0 };

        let row_cfont_y = y;
        let cfont_val = push_stepper_controls(
            &mut controls,
            ControlId::CompactFontDec,
            ControlId::CompactFontInc,
            narrow,
            inner_x,
            inner_w,
            y,
        );
        y += if narrow { 78.0 } else { 49.0 };

        let row_dfont_y = y;
        let dfont_val = push_stepper_controls(
            &mut controls,
            ControlId::DesktopFontDec,
            ControlId::DesktopFontInc,
            narrow,
            inner_x,
            inner_w,
            y,
        );
        y += if narrow { 78.0 } else { 49.0 };

        let row_pos_y = y;
        let status_extra = if self.positions_status.is_some() {
            if narrow { 28.0 } else { 24.0 }
        } else {
            0.0
        };
        let row_pos_base = if narrow { 78.0 } else { 50.0 };
        let row_pos_h = row_pos_base + status_extra;
        {
            let (bx, by, bw) = if narrow {
                (inner_x, row_pos_y + 34.0, inner_w)
            } else {
                (inner_x + inner_w - 136.0, row_pos_y + 7.0, 136.0)
            };
            controls.push(ControlBox {
                id: ControlId::ResetPositions,
                in_header: false,
                vis: RectL::new(bx, by, bw, 36.0),
                hit: RectL::new(bx, by - 4.0, bw, 44.0),
            });
        }
        y += row_pos_h;
        let app_card = RectL::new(pad_x, app_card_y, card_w, y - app_card_y);

        // 2. Timing & Playback
        y += 20.0;
        let sec_timing_y = y;
        y += 26.0;

        let timing_card_y = y;
        let row_offset_y = y;
        let has_offset_reset = self.cfg.ui.offset_ms != 0;
        let (row_offset_h, offset_readout_pos) = if narrow {
            let readout = (inner_x, row_offset_y + 10.0);
            let mut by = row_offset_y + 54.0;
            if has_offset_reset {
                controls.push(ControlBox {
                    id: ControlId::OffsetReset,
                    in_header: false,
                    vis: RectL::new(inner_x, by, inner_w, 36.0),
                    hit: RectL::new(inner_x, by - 4.0, inner_w, 44.0),
                });
                by += 46.0;
            }
            let half_w = (inner_w - 8.0) / 2.0;
            for (row_i, pair) in [
                [ControlId::OffsetM250, ControlId::OffsetM50],
                [ControlId::OffsetP50, ControlId::OffsetP250],
            ]
            .into_iter()
            .enumerate()
            {
                let ry = by + row_i as f32 * 46.0;
                for (col_i, id) in pair.into_iter().enumerate() {
                    let rx = inner_x + col_i as f32 * (half_w + 8.0);
                    controls.push(ControlBox {
                        id,
                        in_header: false,
                        vis: RectL::new(rx, ry, half_w, 36.0),
                        hit: RectL::new(rx, ry - 4.0, half_w, 44.0),
                    });
                }
            }
            let total_h = (by + 92.0) - row_offset_y;
            (total_h, readout)
        } else {
            let readout_right = if has_offset_reset {
                let rx = inner_x + inner_w - 68.0;
                controls.push(ControlBox {
                    id: ControlId::OffsetReset,
                    in_header: false,
                    vis: RectL::new(rx, row_offset_y + 8.0, 68.0, 36.0),
                    hit: RectL::new(rx, row_offset_y + 4.0, 68.0, 44.0),
                });
                rx - 10.0
            } else {
                inner_x + inner_w
            };
            let btn_gap = 8.0;
            let btn_w = (inner_w - 3.0 * btn_gap) / 4.0;
            let by = row_offset_y + 54.0;
            for (i, id) in [
                ControlId::OffsetM250,
                ControlId::OffsetM50,
                ControlId::OffsetP50,
                ControlId::OffsetP250,
            ]
            .into_iter()
            .enumerate()
            {
                let bx = inner_x + i as f32 * (btn_w + btn_gap);
                controls.push(ControlBox {
                    id,
                    in_header: false,
                    vis: RectL::new(bx, by, btn_w, 36.0),
                    hit: RectL::new(bx, by - 4.0, btn_w, 44.0),
                });
            }
            (100.0, (readout_right, row_offset_y + 16.0))
        };
        y += row_offset_h + 1.0;

        let row_media_y = y;
        let media_extra = if self.media_status.is_some() {
            if narrow { 34.0 } else { 24.0 }
        } else {
            0.0
        };
        let row_media_base = if narrow { 78.0 } else { 50.0 };
        let row_media_h = row_media_base + media_extra;
        {
            let (bx, by, bw) = if narrow {
                (inner_x, row_media_y + 34.0, inner_w)
            } else {
                let w_btn = 196.0f32.min(inner_w * 0.52);
                (inner_x + inner_w - w_btn, row_media_y + 7.0, w_btn)
            };
            controls.push(ControlBox {
                id: ControlId::MediaSource,
                in_header: false,
                vis: RectL::new(bx, by, bw, 36.0),
                hit: RectL::new(bx, by - 4.0, bw, 44.0),
            });
        }
        y += row_media_h + 1.0;

        let row_autostart_y = y;
        let auto_extra = if self.autostart_error.is_some() {
            if narrow { 38.0 } else { 28.0 }
        } else {
            0.0
        };
        let row_auto_base = if narrow { 78.0 } else { 48.0 };
        let row_autostart_h = row_auto_base + auto_extra;
        push_toggle_control(
            &mut controls,
            ControlId::ToggleAutostart,
            narrow,
            inner_x,
            inner_w,
            toggle_w,
            row_autostart_y,
        );
        y += row_autostart_h;
        let timing_card = RectL::new(pad_x, timing_card_y, card_w, y - timing_card_y);

        // 3. Local Lyrics Folders (.lrc)
        y += 20.0;
        let sec_lrc_y = y;
        y += 26.0;

        let lrc_card_y = y;
        let mut lrc_empty_rect = None;
        let mut lrc_dir_rows = Vec::with_capacity(self.cfg.lrc_dirs.len());
        if self.cfg.lrc_dirs.is_empty() {
            let eh = if narrow { 86.0 } else { 64.0 };
            lrc_empty_rect = Some(RectL::new(inner_x, y + 10.0, inner_w, eh - 20.0));
            y += eh + 1.0;
        } else {
            for idx in 0..self.cfg.lrc_dirs.len() {
                let ry = y;
                let rh = if narrow { 78.0 } else { 48.0 };
                let (bx, by, bw) = if narrow {
                    (inner_x, ry + 34.0, 96.0)
                } else {
                    (inner_x + inner_w - 84.0, ry + 6.0, 84.0)
                };
                controls.push(ControlBox {
                    id: ControlId::RemoveLrcDir(idx),
                    in_header: false,
                    vis: RectL::new(bx, by, bw, 36.0),
                    hit: RectL::new(bx, by - 4.0, bw, 44.0),
                });
                lrc_dir_rows.push((ry, rh));
                y += rh + 1.0;
            }
        }

        let lrc_form_y = y;
        if narrow {
            let iy = lrc_form_y + 10.0;
            controls.push(ControlBox {
                id: ControlId::LrcInput,
                in_header: false,
                vis: RectL::new(inner_x, iy, inner_w, 38.0),
                hit: RectL::new(inner_x, iy - 3.0, inner_w, 44.0),
            });
            let by = lrc_form_y + 56.0;
            controls.push(ControlBox {
                id: ControlId::AddLrcDir,
                in_header: false,
                vis: RectL::new(inner_x, by, inner_w, 38.0),
                hit: RectL::new(inner_x, by - 3.0, inner_w, 44.0),
            });
            y += 104.0;
        } else {
            let add_w = 108.0;
            let inp_w = (inner_w - add_w - 8.0).max(120.0);
            let iy = lrc_form_y + 10.0;
            controls.push(ControlBox {
                id: ControlId::LrcInput,
                in_header: false,
                vis: RectL::new(inner_x, iy, inp_w, 38.0),
                hit: RectL::new(inner_x, iy - 3.0, inp_w, 44.0),
            });
            controls.push(ControlBox {
                id: ControlId::AddLrcDir,
                in_header: false,
                vis: RectL::new(inner_x + inp_w + 8.0, iy, add_w, 38.0),
                hit: RectL::new(inner_x + inp_w + 8.0, iy - 3.0, add_w, 44.0),
            });
            y += 58.0;
        }

        let lrc_feedback_y = if self.lrc_feedback.is_some() {
            let fy = y;
            y += if narrow { 38.0 } else { 28.0 };
            Some(fy)
        } else {
            None
        };
        let lrc_card = RectL::new(pad_x, lrc_card_y, card_w, y - lrc_card_y);

        // 4. Global Hotkeys
        y += 20.0;
        let sec_hotkeys_y = y;
        y += 26.0;

        let hotkeys_card_y = y;
        let mut hotkey_rows = [(0.0, 0.0); 7];
        for (idx, slot) in hotkey_rows.iter_mut().enumerate() {
            let ry = y;
            let is_rec = self.recording_hotkey == Some(idx);
            let stacked = narrow || is_rec;
            let err_extra = if self.hotkey_error_for(idx).is_some() {
                if narrow { 34.0 } else { 26.0 }
            } else {
                0.0
            };
            let base_h = if stacked { 78.0 } else { 48.0 };
            let rh = base_h + err_extra;
            let (bx, by, bw) = if stacked {
                (inner_x, ry + 34.0, inner_w)
            } else {
                (inner_x + inner_w - 168.0, ry + 6.0, 168.0)
            };
            controls.push(ControlBox {
                id: ControlId::Hotkey(idx),
                in_header: false,
                vis: RectL::new(bx, by, bw, 36.0),
                hit: RectL::new(bx, by - 4.0, bw, 44.0),
            });
            *slot = (ry, rh);
            y += rh + 1.0;
        }

        let restore_row_y = y;
        let restore_extra = if self.hotkey_status.is_some() {
            if narrow { 32.0 } else { 24.0 }
        } else {
            0.0
        };
        let restore_w = if narrow { inner_w } else { 212.0 };
        controls.push(ControlBox {
            id: ControlId::RestoreHotkeys,
            in_header: false,
            vis: RectL::new(inner_x, restore_row_y + 9.0, restore_w, 36.0),
            hit: RectL::new(inner_x, restore_row_y + 5.0, restore_w, 44.0),
        });
        y += 54.0 + restore_extra;
        let hotkeys_card = RectL::new(pad_x, hotkeys_card_y, card_w, y - hotkeys_card_y);

        let content_h = y + 24.0;

        Layout {
            narrow,
            pad_x,
            card_w,
            inner_x,
            inner_w,
            content_h,
            controls,
            sec_appearance_y,
            theme_card,
            theme_label_y,
            app_card,
            row_island_y,
            row_desktop_y,
            row_karaoke_y,
            row_click_y,
            row_click_h,
            row_max_w_y,
            max_w_val,
            row_cfont_y,
            cfont_val,
            row_dfont_y,
            dfont_val,
            row_pos_y,
            row_pos_h,
            sec_timing_y,
            timing_card,
            row_offset_y,
            row_offset_h,
            offset_readout_pos,
            row_media_y,
            row_media_h,
            row_autostart_y,
            row_autostart_h,
            sec_lrc_y,
            lrc_card,
            lrc_empty_rect,
            lrc_dir_rows,
            lrc_form_y,
            lrc_feedback_y,
            sec_hotkeys_y,
            hotkeys_card,
            hotkey_rows,
            restore_row_y,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_header(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        layout: &Layout,
        scale: f32,
        win_w: f32,
        header_h_px: f32,
    ) {
        let pw = pm.width() as f32;
        fill_rect_px(pm, 0.0, 0.0, pw, header_h_px, pal.bg_rgba, None);
        fill_rect_px(
            pm,
            0.0,
            (header_h_px - scale.max(1.0)).max(0.0),
            pw,
            scale.max(1.0),
            pal.divider,
            None,
        );

        let close_box = layout.find(ControlId::Close).copied();
        let text_right = close_box.map_or(win_w - layout.pad_x, |c| c.hit.x - 8.0);
        let max_text_w = ((text_right - layout.pad_x).max(80.0)) * scale;
        let hdr_clip: Clip = (0.0, 0.0, pw, header_h_px);

        let title_lbl = self.labels.next("Lyrics Island Settings");
        title_lbl.layout(t, HEADER_TITLE, scale, max_text_w, 1);
        t.draw(
            pm,
            title_lbl,
            ((layout.pad_x * scale).round(), (9.0 * scale).round()),
            pal.text_primary,
            hdr_clip,
            None,
        );

        let status_copy = self.status_line.clone();
        let sub_lbl = self.labels.next(&status_copy);
        sub_lbl.layout(t, HEADER_SUB, scale, max_text_w, 1);
        t.draw(
            pm,
            sub_lbl,
            ((layout.pad_x * scale).round(), (31.0 * scale).round()),
            pal.text_secondary,
            hdr_clip,
            None,
        );

        if let Some(cb) = close_box {
            let vr = cb.vis.scaled(scale, 0.0);
            let hovered = self.hover == Some(ControlId::Close);
            let focused = self.focus == Some(ControlId::Close);
            let bg = if hovered { pal.hover_rgba } else { pal.elevated_rgba };
            fill_rrect(pm, vr, 8.0 * scale, bg, None);
            stroke_rrect(pm, vr, 8.0 * scale, 1.0 * scale, pal.ctrl_border, None);
            draw_close_icon(pm, vr.x + vr.w / 2.0, vr.y + vr.h / 2.0, scale, pal.text_primary);
            if focused {
                draw_focus_ring(pm, cb.vis, scale, 0.0, 8.0, pal.accent, None);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_appearance(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        layout: &Layout,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        self.draw_section_heading(pm, t, pal, "Appearance", layout.pad_x, layout.sec_appearance_y, layout.card_w, scale, dy, clip);

        // Theme card
        draw_card(pm, layout.theme_card.scaled(scale, dy), scale, pal, mask);
        self.draw_body_label(
            pm,
            t,
            "Theme",
            ROW_TITLE,
            pal.text_primary,
            layout.inner_x,
            layout.theme_label_y,
            if layout.narrow { layout.inner_w } else { layout.inner_w - 250.0 },
            1,
            scale,
            dy,
            clip,
        );
        for (id, name, theme) in [
            (ControlId::ThemeSystem, "System", Theme::System),
            (ControlId::ThemeDark, "Dark", Theme::Dark),
            (ControlId::ThemeLight, "Light", Theme::Light),
        ] {
            if let Some(cb) = layout.find(id) {
                let active = self.cfg.ui.theme == theme;
                self.draw_segment_btn(pm, t, pal, cb, name, active, scale, dy, clip, mask);
            }
        }

        // Grouped Appearance controls card
        draw_card(pm, layout.app_card.scaled(scale, dy), scale, pal, mask);

        self.draw_toggle_row(
            pm,
            t,
            pal,
            layout,
            "Show island",
            layout.row_island_y,
            ControlId::ToggleIsland,
            self.cfg.ui.island_visible,
            scale,
            dy,
            clip,
            mask,
        );
        draw_divider(pm, layout, layout.row_desktop_y - 1.0, scale, dy, pal, mask);

        self.draw_toggle_row(
            pm,
            t,
            pal,
            layout,
            "Desktop lyrics overlay",
            layout.row_desktop_y,
            ControlId::ToggleDesktop,
            self.cfg.ui.desktop_visible,
            scale,
            dy,
            clip,
            mask,
        );
        draw_divider(pm, layout, layout.row_karaoke_y - 1.0, scale, dy, pal, mask);

        self.draw_toggle_row(
            pm,
            t,
            pal,
            layout,
            "Karaoke word highlight",
            layout.row_karaoke_y,
            ControlId::ToggleKaraoke,
            self.cfg.ui.karaoke,
            scale,
            dy,
            clip,
            mask,
        );
        draw_divider(pm, layout, layout.row_click_y - 1.0, scale, dy, pal, mask);

        // Click-through row with live hotkey subtitle
        let click_sub = self.click_through_subtitle();
        let text_max_w = if layout.narrow { layout.inner_w } else { (layout.inner_w - 98.0).max(100.0) };
        self.draw_body_label(
            pm,
            t,
            "Click-through overlays",
            ROW_TITLE,
            pal.text_primary,
            layout.inner_x,
            layout.row_click_y + 10.0,
            text_max_w,
            1,
            scale,
            dy,
            clip,
        );
        self.draw_body_label(
            pm,
            t,
            &click_sub,
            ROW_SUB,
            pal.text_secondary,
            layout.inner_x,
            layout.row_click_y + 30.0,
            text_max_w,
            2,
            scale,
            dy,
            clip,
        );
        if let Some(cb) = layout.find(ControlId::ToggleClickThrough) {
            self.draw_switch(pm, t, pal, cb, self.cfg.ui.click_through, scale, dy, clip, mask);
        }
        draw_divider(pm, layout, layout.row_click_y + layout.row_click_h, scale, dy, pal, mask);

        // Steppers
        let max_w_str = format!("{} px", self.cfg.ui.island_max_width.round() as i32);
        self.draw_stepper_row(
            pm,
            t,
            pal,
            layout,
            "Max compact width",
            layout.row_max_w_y,
            layout.max_w_val,
            &max_w_str,
            ControlId::MaxWidthDec,
            ControlId::MaxWidthInc,
            self.cfg.ui.island_max_width <= 200.0,
            self.cfg.ui.island_max_width >= 1400.0,
            scale,
            dy,
            clip,
            mask,
        );
        draw_divider(pm, layout, layout.row_cfont_y - 1.0, scale, dy, pal, mask);

        let cfont_str = format!("{} pt", self.cfg.ui.island_font_pt.round() as i32);
        self.draw_stepper_row(
            pm,
            t,
            pal,
            layout,
            "Compact font size",
            layout.row_cfont_y,
            layout.cfont_val,
            &cfont_str,
            ControlId::CompactFontDec,
            ControlId::CompactFontInc,
            self.cfg.ui.island_font_pt <= 8.0,
            self.cfg.ui.island_font_pt >= 18.0,
            scale,
            dy,
            clip,
            mask,
        );
        draw_divider(pm, layout, layout.row_dfont_y - 1.0, scale, dy, pal, mask);

        let dfont_str = format!("{} px", self.cfg.ui.desktop_font_px.round() as i32);
        self.draw_stepper_row(
            pm,
            t,
            pal,
            layout,
            "Desktop lyrics font size",
            layout.row_dfont_y,
            layout.dfont_val,
            &dfont_str,
            ControlId::DesktopFontDec,
            ControlId::DesktopFontInc,
            self.cfg.ui.desktop_font_px <= 16.0,
            self.cfg.ui.desktop_font_px >= 96.0,
            scale,
            dy,
            clip,
            mask,
        );
        draw_divider(pm, layout, layout.row_pos_y - 1.0, scale, dy, pal, mask);

        // Window placement
        let pos_title_y = if layout.narrow { layout.row_pos_y + 10.0 } else { layout.row_pos_y + 16.0 };
        let pos_title_w = if layout.narrow { layout.inner_w } else { layout.inner_w - 148.0 };
        self.draw_body_label(
            pm,
            t,
            "Window placement",
            ROW_TITLE,
            pal.text_primary,
            layout.inner_x,
            pos_title_y,
            pos_title_w,
            1,
            scale,
            dy,
            clip,
        );
        if let Some(cb) = layout.find(ControlId::ResetPositions) {
            self.draw_button(pm, t, pal, cb, "Reset positions", false, scale, dy, clip, mask);
        }
        if let Some(status) = self.positions_status.clone() {
            let sy = layout.row_pos_y + layout.row_pos_h - if layout.narrow { 24.0 } else { 21.0 };
            self.draw_body_label(
                pm,
                t,
                &status,
                ROW_SUB,
                pal.success,
                layout.inner_x,
                sy,
                layout.inner_w,
                1,
                scale,
                dy,
                clip,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_timing(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        layout: &Layout,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        self.draw_section_heading(
            pm,
            t,
            pal,
            "Timing & Playback",
            layout.pad_x,
            layout.sec_timing_y,
            layout.card_w,
            scale,
            dy,
            clip,
        );
        draw_card(pm, layout.timing_card.scaled(scale, dy), scale, pal, mask);

        // Lyrics offset
        let offset_str = fmt_offset_ms(self.cfg.ui.offset_ms);
        if layout.narrow {
            self.draw_body_label(
                pm,
                t,
                "Lyrics offset",
                ROW_TITLE,
                pal.text_primary,
                layout.inner_x,
                layout.row_offset_y + 10.0,
                layout.inner_w - 90.0,
                1,
                scale,
                dy,
                clip,
            );
            let ro_lbl = self.labels.next(&offset_str);
            ro_lbl.layout(t, VALUE_TEXT, scale, 90.0 * scale, 1);
            let rx = (layout.inner_x + layout.inner_w) * scale - ro_lbl.width();
            let ry = (layout.row_offset_y + 10.0 + dy) * scale;
            t.draw(pm, ro_lbl, (rx.round(), ry.round()), pal.text_primary, clip, None);
            self.draw_body_label(
                pm,
                t,
                "Positive values show lines earlier",
                ROW_SUB,
                pal.text_secondary,
                layout.inner_x,
                layout.row_offset_y + 30.0,
                layout.inner_w,
                1,
                scale,
                dy,
                clip,
            );
        } else {
            self.draw_body_label(
                pm,
                t,
                "Lyrics offset",
                ROW_TITLE,
                pal.text_primary,
                layout.inner_x,
                layout.row_offset_y + 9.0,
                layout.inner_w - 170.0,
                1,
                scale,
                dy,
                clip,
            );
            self.draw_body_label(
                pm,
                t,
                "Positive values show lines earlier",
                ROW_SUB,
                pal.text_secondary,
                layout.inner_x,
                layout.row_offset_y + 28.0,
                layout.inner_w - 170.0,
                1,
                scale,
                dy,
                clip,
            );
            let ro_lbl = self.labels.next(&offset_str);
            ro_lbl.layout(t, VALUE_TEXT, scale, 110.0 * scale, 1);
            let rx = layout.offset_readout_pos.0 * scale - ro_lbl.width();
            let ry = (layout.offset_readout_pos.1 + dy) * scale;
            t.draw(pm, ro_lbl, (rx.round(), ry.round()), pal.text_primary, clip, None);
        }

        if let Some(cb) = layout.find(ControlId::OffsetReset) {
            self.draw_button(pm, t, pal, cb, "Reset", false, scale, dy, clip, mask);
        }
        for (id, label, muted) in [
            (ControlId::OffsetM250, "\u{2212}250 ms", self.cfg.ui.offset_ms <= -30_000),
            (ControlId::OffsetM50, "\u{2212}50 ms", self.cfg.ui.offset_ms <= -30_000),
            (ControlId::OffsetP50, "+50 ms", self.cfg.ui.offset_ms >= 30_000),
            (ControlId::OffsetP250, "+250 ms", self.cfg.ui.offset_ms >= 30_000),
        ] {
            if let Some(cb) = layout.find(id) {
                self.draw_button(pm, t, pal, cb, label, muted, scale, dy, clip, mask);
            }
        }
        draw_divider(pm, layout, layout.row_offset_y + layout.row_offset_h, scale, dy, pal, mask);

        // Media source
        let media_title_y = if layout.narrow { layout.row_media_y + 10.0 } else { layout.row_media_y + 16.0 };
        let media_title_w = if layout.narrow { layout.inner_w } else { layout.inner_w * 0.45 };
        self.draw_body_label(
            pm,
            t,
            "Media source",
            ROW_TITLE,
            pal.text_primary,
            layout.inner_x,
            media_title_y,
            media_title_w,
            1,
            scale,
            dy,
            clip,
        );
        let source_name = self.media_source_label();
        if let Some(cb) = layout.find(ControlId::MediaSource) {
            self.draw_selector_btn(pm, t, pal, cb, &source_name, scale, dy, clip, mask);
        }
        if let Some(note) = self.media_status.clone() {
            let ny = layout.row_media_y + layout.row_media_h - if layout.narrow { 30.0 } else { 21.0 };
            self.draw_body_label(
                pm,
                t,
                &note,
                ROW_SUB,
                pal.text_secondary,
                layout.inner_x,
                ny,
                layout.inner_w,
                2,
                scale,
                dy,
                clip,
            );
        }
        draw_divider(pm, layout, layout.row_media_y + layout.row_media_h, scale, dy, pal, mask);

        // Launch at login
        self.draw_toggle_row(
            pm,
            t,
            pal,
            layout,
            "Launch at login",
            layout.row_autostart_y,
            ControlId::ToggleAutostart,
            self.autostart,
            scale,
            dy,
            clip,
            mask,
        );
        if let Some(err) = self.autostart_error.clone() {
            let msg = prefix_error(&err);
            let ey = layout.row_autostart_y + layout.row_autostart_h - if layout.narrow { 34.0 } else { 24.0 };
            self.draw_body_label(
                pm,
                t,
                &msg,
                ROW_SUB,
                pal.error,
                layout.inner_x,
                ey,
                layout.inner_w,
                2,
                scale,
                dy,
                clip,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_lrc_dirs(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        layout: &Layout,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        self.draw_section_heading(
            pm,
            t,
            pal,
            "Local Lyrics Folders (.lrc)",
            layout.pad_x,
            layout.sec_lrc_y,
            layout.card_w,
            scale,
            dy,
            clip,
        );
        draw_card(pm, layout.lrc_card.scaled(scale, dy), scale, pal, mask);

        if let Some(empty_r) = layout.lrc_empty_rect {
            self.draw_body_label(
                pm,
                t,
                LRC_EMPTY_STATE,
                ROW_SUB,
                pal.text_secondary,
                empty_r.x,
                empty_r.y,
                empty_r.w,
                4,
                scale,
                dy,
                clip,
            );
            draw_divider(pm, layout, layout.lrc_form_y - 1.0, scale, dy, pal, mask);
        } else {
            let dirs: Vec<String> =
                self.cfg.lrc_dirs.iter().map(|p| p.display().to_string()).collect();
            for (idx, path_str) in dirs.iter().enumerate() {
                let Some(&(ry, rh)) = layout.lrc_dir_rows.get(idx) else { continue };
                let path_y = if layout.narrow { ry + 10.0 } else { ry + 15.0 };
                let path_w = if layout.narrow { layout.inner_w } else { (layout.inner_w - 96.0).max(80.0) };
                self.draw_body_label(
                    pm,
                    t,
                    path_str,
                    INPUT_TEXT,
                    pal.text_primary,
                    layout.inner_x,
                    path_y,
                    path_w,
                    1,
                    scale,
                    dy,
                    clip,
                );
                if let Some(cb) = layout.find(ControlId::RemoveLrcDir(idx)) {
                    self.draw_button(pm, t, pal, cb, "Remove", false, scale, dy, clip, mask);
                }
                draw_divider(pm, layout, ry + rh, scale, dy, pal, mask);
            }
        }

        if let Some(inp_cb) = layout.find(ControlId::LrcInput) {
            self.draw_text_input(pm, t, pal, inp_cb, scale, dy, clip, mask);
        }
        if let Some(add_cb) = layout.find(ControlId::AddLrcDir) {
            self.draw_button(pm, t, pal, add_cb, "Add folder", false, scale, dy, clip, mask);
        }

        if let (Some(fy), Some(fb)) = (layout.lrc_feedback_y, self.lrc_feedback.clone()) {
            let (msg, col) = match fb {
                Feedback::Status(s) => (s, pal.success),
                Feedback::Error(s) => (s, pal.error),
            };
            self.draw_body_label(
                pm,
                t,
                &msg,
                ROW_SUB,
                col,
                layout.inner_x,
                fy,
                layout.inner_w,
                2,
                scale,
                dy,
                clip,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_hotkeys(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        layout: &Layout,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        self.draw_section_heading(
            pm,
            t,
            pal,
            "Global Hotkeys",
            layout.pad_x,
            layout.sec_hotkeys_y,
            layout.card_w,
            scale,
            dy,
            clip,
        );
        draw_card(pm, layout.hotkeys_card.scaled(scale, dy), scale, pal, mask);

        let bindings: [String; 7] = [
            self.cfg.hotkeys.toggle_island.clone(),
            self.cfg.hotkeys.toggle_desktop.clone(),
            self.cfg.hotkeys.toggle_click_through.clone(),
            self.cfg.hotkeys.play_pause.clone(),
            self.cfg.hotkeys.offset_earlier.clone(),
            self.cfg.hotkeys.offset_later.clone(),
            self.cfg.hotkeys.offset_reset.clone(),
        ];

        for idx in 0..7 {
            let (ry, rh) = layout.hotkey_rows[idx];
            let is_rec = self.recording_hotkey == Some(idx);
            let stacked = layout.narrow || is_rec;
            let title_y = if stacked { ry + 10.0 } else { ry + 15.0 };
            let title_w = if stacked { layout.inner_w } else { (layout.inner_w - 180.0).max(80.0) };
            self.draw_body_label(
                pm,
                t,
                HOTKEY_LABELS[idx],
                ROW_TITLE,
                pal.text_primary,
                layout.inner_x,
                title_y,
                title_w,
                1,
                scale,
                dy,
                clip,
            );

            if let Some(cb) = layout.find(ControlId::Hotkey(idx)) {
                self.draw_hotkey_btn(pm, t, pal, cb, &bindings[idx], is_rec, scale, dy, clip, mask);
            }

            if let Some(reason) = self.hotkey_error_for(idx).map(str::to_owned) {
                let err_msg = prefix_error(&reason);
                let ey = ry + rh - if layout.narrow { 30.0 } else { 22.0 };
                self.draw_body_label(
                    pm,
                    t,
                    &err_msg,
                    ROW_SUB,
                    pal.error,
                    layout.inner_x,
                    ey,
                    layout.inner_w,
                    2,
                    scale,
                    dy,
                    clip,
                );
            }
            draw_divider(pm, layout, ry + rh, scale, dy, pal, mask);
        }

        if let Some(cb) = layout.find(ControlId::RestoreHotkeys) {
            self.draw_button(
                pm,
                t,
                pal,
                cb,
                "Restore default shortcuts",
                false,
                scale,
                dy,
                clip,
                mask,
            );
        }
        if let Some(status) = self.hotkey_status.clone() {
            let sy = layout.restore_row_y + 50.0;
            self.draw_body_label(
                pm,
                t,
                &status,
                ROW_SUB,
                pal.success,
                layout.inner_x,
                sy,
                layout.inner_w,
                1,
                scale,
                dy,
                clip,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_section_heading(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        title: &str,
        x: f32,
        y: f32,
        max_w: f32,
        scale: f32,
        dy: f32,
        clip: Clip,
    ) {
        self.draw_body_label(pm, t, title, SECTION_HEAD, pal.text_primary, x, y, max_w, 1, scale, dy, clip);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_body_label(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        text: &str,
        st: Style,
        color: Rgba,
        x: f32,
        y: f32,
        max_w: f32,
        lines: usize,
        scale: f32,
        dy: f32,
        clip: Clip,
    ) {
        let py = (y + dy) * scale;
        let est_h = st.line * (lines.max(1) as f32) * scale;
        if py + est_h < clip.1 || py > clip.3 {
            return;
        }
        let lbl = self.labels.next(text);
        lbl.layout(t, st, scale, (max_w * scale).max(1.0), lines);
        let px = (x * scale).round();
        let item_clip = (clip.0.max(px), clip.1, clip.2.min(px + max_w * scale), clip.3);
        t.draw(pm, lbl, (px, py.round()), color, item_clip, None);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_toggle_row(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        layout: &Layout,
        title: &str,
        row_y: f32,
        id: ControlId,
        on: bool,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        let title_y = if layout.narrow { row_y + 10.0 } else { row_y + 15.0 };
        let title_w = if layout.narrow { layout.inner_w } else { (layout.inner_w - 94.0).max(80.0) };
        self.draw_body_label(
            pm,
            t,
            title,
            ROW_TITLE,
            pal.text_primary,
            layout.inner_x,
            title_y,
            title_w,
            1,
            scale,
            dy,
            clip,
        );
        if let Some(cb) = layout.find(id) {
            self.draw_switch(pm, t, pal, cb, on, scale, dy, clip, mask);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_switch(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        cb: &ControlBox,
        on: bool,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        let vr = cb.vis.scaled(scale, dy);
        if vr.y + vr.h < clip.1 || vr.y > clip.3 {
            return;
        }
        let hovered = self.hover == Some(cb.id);
        let focused = self.focus == Some(cb.id);

        if hovered {
            fill_rrect(pm, vr, 8.0 * scale, pal.hover_rgba, mask);
        }

        // Pair color with an explicit On/Off text state so status is never color-only.
        let state_str = if on { "On" } else { "Off" };
        let state_col = if on { pal.text_primary } else { pal.text_secondary };
        let lbl = self.labels.next(state_str);
        lbl.layout(t, CTRL_TEXT, scale, 32.0 * scale, 1);
        let lx = vr.x + 6.0 * scale;
        let ly = vr.y + (vr.h - CTRL_TEXT.line * scale) / 2.0;
        t.draw(pm, lbl, (lx.round(), ly.round()), state_col, clip, None);

        let track_w = 44.0 * scale;
        let track_h = 24.0 * scale;
        let track_x = vr.x + vr.w - track_w - 2.0 * scale;
        let track_y = vr.y + (vr.h - track_h) / 2.0;
        let track = RectL::new(track_x, track_y, track_w, track_h);

        if on {
            fill_rrect(pm, track, track_h / 2.0, pal.accent, mask);
        } else {
            fill_rrect(pm, track, track_h / 2.0, pal.switch_off_bg, mask);
            stroke_rrect(pm, track, track_h / 2.0, 1.2 * scale, pal.ctrl_border, mask);
        }

        let knob_r = 9.0 * scale;
        let knob_cx = if on { track_x + track_w - 12.0 * scale } else { track_x + 12.0 * scale };
        let knob_cy = track_y + track_h / 2.0;
        let knob_col = if on { pal.on_accent } else { pal.switch_off_knob };
        if let Some(circle) = PathBuilder::from_circle(knob_cx, knob_cy, knob_r) {
            fill_path(pm, &circle, knob_col, mask);
        }

        if focused {
            draw_focus_ring(pm, cb.vis, scale, dy, 8.0, pal.accent, mask);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_stepper_row(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        layout: &Layout,
        title: &str,
        row_y: f32,
        val_rect: RectL,
        val_text: &str,
        dec_id: ControlId,
        inc_id: ControlId,
        at_min: bool,
        at_max: bool,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        let title_y = if layout.narrow { row_y + 10.0 } else { row_y + 15.0 };
        let title_w = if layout.narrow { layout.inner_w } else { (layout.inner_w - 180.0).max(80.0) };
        self.draw_body_label(
            pm,
            t,
            title,
            ROW_TITLE,
            pal.text_primary,
            layout.inner_x,
            title_y,
            title_w,
            1,
            scale,
            dy,
            clip,
        );
        if let Some(cb) = layout.find(dec_id) {
            self.draw_button(pm, t, pal, cb, "\u{2212}", at_min, scale, dy, clip, mask);
        }
        let vr = val_rect.scaled(scale, dy);
        if vr.y + vr.h >= clip.1 && vr.y <= clip.3 {
            let lbl = self.labels.next(val_text);
            lbl.layout(t, VALUE_TEXT, scale, vr.w.max(1.0), 1);
            let tx = vr.x + ((vr.w - lbl.width()) / 2.0).max(0.0);
            let ty = vr.y + (vr.h - VALUE_TEXT.line * scale) / 2.0;
            t.draw(pm, lbl, (tx.round(), ty.round()), pal.text_primary, clip, None);
        }
        if let Some(cb) = layout.find(inc_id) {
            self.draw_button(pm, t, pal, cb, "+", at_max, scale, dy, clip, mask);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_segment_btn(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        cb: &ControlBox,
        label: &str,
        active: bool,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        let vr = cb.vis.scaled(scale, dy);
        if vr.y + vr.h < clip.1 || vr.y > clip.3 {
            return;
        }
        let hovered = self.hover == Some(cb.id);
        let focused = self.focus == Some(cb.id);

        if active {
            fill_rrect(pm, vr, 6.0 * scale, pal.accent, mask);
        } else {
            let bg = if hovered { pal.hover_rgba } else { pal.bg_rgba };
            fill_rrect(pm, vr, 6.0 * scale, bg, mask);
            stroke_rrect(pm, vr, 6.0 * scale, 1.1 * scale, pal.ctrl_border, mask);
        }

        let ink = if active { pal.on_accent } else { pal.text_primary };
        let lbl = self.labels.next(label);
        lbl.layout(t, CTRL_TEXT, scale, (vr.w - 10.0 * scale).max(1.0), 1);
        let tx = vr.x + ((vr.w - lbl.width()) / 2.0).max(4.0 * scale);
        let ty = vr.y + (vr.h - CTRL_TEXT.line * scale) / 2.0;
        t.draw(pm, lbl, (tx.round(), ty.round()), ink, clip, None);

        if focused {
            draw_focus_ring(pm, cb.vis, scale, dy, 6.0, pal.accent, mask);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_button(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        cb: &ControlBox,
        label: &str,
        muted: bool,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        let vr = cb.vis.scaled(scale, dy);
        if vr.y + vr.h < clip.1 || vr.y > clip.3 {
            return;
        }
        let hovered = self.hover == Some(cb.id) && !muted;
        let focused = self.focus == Some(cb.id);

        let bg = if hovered { pal.hover_rgba } else { pal.bg_rgba };
        let border = if muted { pal.divider } else { pal.ctrl_border };
        let ink = if muted { pal.text_secondary } else { pal.text_primary };

        fill_rrect(pm, vr, 6.0 * scale, bg, mask);
        stroke_rrect(pm, vr, 6.0 * scale, 1.1 * scale, border, mask);

        let pad = 8.0 * scale;
        let lbl = self.labels.next(label);
        lbl.layout(t, CTRL_TEXT, scale, (vr.w - 2.0 * pad).max(1.0), 1);
        let tx = vr.x + ((vr.w - lbl.width()) / 2.0).max(pad);
        let ty = vr.y + (vr.h - CTRL_TEXT.line * scale) / 2.0;
        let btn_clip = (clip.0.max(vr.x + 4.0 * scale), clip.1, clip.2.min(vr.x + vr.w - 4.0 * scale), clip.3);
        t.draw(pm, lbl, (tx.round(), ty.round()), ink, btn_clip, None);

        if focused {
            draw_focus_ring(pm, cb.vis, scale, dy, 6.0, pal.accent, mask);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_selector_btn(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        cb: &ControlBox,
        value: &str,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        let vr = cb.vis.scaled(scale, dy);
        if vr.y + vr.h < clip.1 || vr.y > clip.3 {
            return;
        }
        let hovered = self.hover == Some(cb.id);
        let focused = self.focus == Some(cb.id);
        let bg = if hovered { pal.hover_rgba } else { pal.bg_rgba };

        fill_rrect(pm, vr, 6.0 * scale, bg, mask);
        stroke_rrect(pm, vr, 6.0 * scale, 1.1 * scale, pal.ctrl_border, mask);

        let pad_l = 10.0 * scale;
        let chevron_space = 22.0 * scale;
        let avail_w = (vr.w - pad_l - chevron_space).max(1.0);
        let lbl = self.labels.next(value);
        lbl.layout(t, CTRL_TEXT, scale, avail_w, 1);
        let ty = vr.y + (vr.h - CTRL_TEXT.line * scale) / 2.0;
        let btn_clip = (clip.0.max(vr.x + pad_l), clip.1, clip.2.min(vr.x + vr.w - chevron_space), clip.3);
        t.draw(pm, lbl, ((vr.x + pad_l).round(), ty.round()), pal.text_primary, btn_clip, None);

        // Crisp opposing cycle chevrons on the right edge of the selector.
        draw_selector_arrows(pm, vr.x + vr.w - 12.0 * scale, vr.y + vr.h / 2.0, scale, pal.text_secondary, mask);

        if focused {
            draw_focus_ring(pm, cb.vis, scale, dy, 6.0, pal.accent, mask);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_hotkey_btn(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        cb: &ControlBox,
        accel: &str,
        recording: bool,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        let vr = cb.vis.scaled(scale, dy);
        if vr.y + vr.h < clip.1 || vr.y > clip.3 {
            return;
        }
        let hovered = self.hover == Some(cb.id);
        let focused = self.focus == Some(cb.id);

        let bg = if recording || hovered { pal.hover_rgba } else { pal.bg_rgba };
        let (border_col, border_w) = if recording {
            (pal.accent, 1.8 * scale)
        } else {
            (pal.ctrl_border, 1.1 * scale)
        };
        fill_rrect(pm, vr, 6.0 * scale, bg, mask);
        stroke_rrect(pm, vr, 6.0 * scale, border_w, border_col, mask);

        let trimmed = accel.trim();
        let (text, ink) = if recording {
            (CAPTURE_PROMPT, pal.accent)
        } else if trimmed.is_empty() {
            ("Disabled", pal.text_secondary)
        } else {
            (trimmed, pal.text_primary)
        };

        let pad = 8.0 * scale;
        let lbl = self.labels.next(text);
        lbl.layout(t, CTRL_TEXT, scale, (vr.w - 2.0 * pad).max(1.0), 1);
        let tx = vr.x + ((vr.w - lbl.width()) / 2.0).max(pad);
        let ty = vr.y + (vr.h - CTRL_TEXT.line * scale) / 2.0;
        let btn_clip = (clip.0.max(vr.x + 4.0 * scale), clip.1, clip.2.min(vr.x + vr.w - 4.0 * scale), clip.3);
        t.draw(pm, lbl, (tx.round(), ty.round()), ink, btn_clip, None);

        if focused && !recording {
            draw_focus_ring(pm, cb.vis, scale, dy, 6.0, pal.accent, mask);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_text_input(
        &mut self,
        pm: &mut Pixmap,
        t: &mut Text,
        pal: &Palette,
        cb: &ControlBox,
        scale: f32,
        dy: f32,
        clip: Clip,
        mask: Option<&Mask>,
    ) {
        let vr = cb.vis.scaled(scale, dy);
        if vr.y + vr.h < clip.1 || vr.y > clip.3 {
            return;
        }
        let focused = self.focus == Some(cb.id);
        let hovered = self.hover == Some(cb.id);
        let has_err = matches!(self.lrc_feedback, Some(Feedback::Error(_)));

        fill_rrect(pm, vr, 6.0 * scale, pal.bg_rgba, mask);
        let (border_col, border_w) = if focused {
            (pal.accent, 2.0 * scale)
        } else if has_err {
            (pal.error, 1.5 * scale)
        } else if hovered {
            (pal.text_secondary, 1.2 * scale)
        } else {
            (pal.ctrl_border, 1.1 * scale)
        };
        stroke_rrect(pm, vr, 6.0 * scale, border_w, border_col, mask);

        let inner_x0 = vr.x + 10.0 * scale;
        let inner_x1 = (vr.x + vr.w - 10.0 * scale).max(inner_x0 + 1.0);
        let avail = inner_x1 - inner_x0;
        let inp_clip: Clip = (clip.0.max(inner_x0), clip.1, clip.2.min(inner_x1), clip.3);
        let ty = vr.y + (vr.h - INPUT_TEXT.line * scale) / 2.0;

        if self.folder_input.is_empty() {
            let caret_offset = if focused { 4.0 * scale } else { 0.0 };
            let lbl = self.labels.next(lrc_placeholder());
            lbl.layout(t, INPUT_TEXT, scale, (avail - caret_offset).max(1.0), 1);
            t.draw(
                pm,
                lbl,
                ((inner_x0 + caret_offset).round(), ty.round()),
                pal.text_secondary,
                inp_clip,
                None,
            );
            if focused {
                fill_rect_px(
                    pm,
                    inner_x0.round(),
                    (vr.y + 9.0 * scale).round(),
                    (1.8 * scale).max(1.0),
                    (vr.h - 18.0 * scale).max(10.0),
                    pal.accent,
                    mask,
                );
            }
        } else {
            let typed = self.folder_input.clone();
            let lbl = self.labels.next(&typed);
            if focused {
                // Keep the end of long paths and the caret visible while typing.
                lbl.layout(t, INPUT_TEXT, scale, 4096.0 * scale, 1);
                let w = lbl.width();
                let tx = if w + 4.0 * scale <= avail {
                    inner_x0
                } else {
                    inner_x1 - w - 4.0 * scale
                };
                t.draw(pm, lbl, (tx.round(), ty.round()), pal.text_primary, inp_clip, None);
                let caret_x = (tx + w + 1.5 * scale).min(inner_x1 - 2.0 * scale);
                fill_rect_px(
                    pm,
                    caret_x.round(),
                    (vr.y + 9.0 * scale).round(),
                    (1.8 * scale).max(1.0),
                    (vr.h - 18.0 * scale).max(10.0),
                    pal.accent,
                    mask,
                );
            } else {
                lbl.layout(t, INPUT_TEXT, scale, avail, 1);
                t.draw(pm, lbl, (inner_x0.round(), ty.round()), pal.text_primary, inp_clip, None);
            }
        }
    }
}

fn sanitize_w(win_w: f32) -> f32 {
    if win_w.is_finite() && win_w > 0.0 {
        win_w
    } else {
        DEFAULT_W
    }
}

fn close_hit_rect(win_w: f32) -> RectL {
    RectL::new((win_w - 50.0).max(0.0), 6.0, 44.0, 44.0)
}

fn push_toggle_control(
    controls: &mut Vec<ControlBox>,
    id: ControlId,
    narrow: bool,
    inner_x: f32,
    inner_w: f32,
    toggle_w: f32,
    row_y: f32,
) {
    let (vx, vy) = if narrow {
        (inner_x, row_y + 34.0)
    } else {
        (inner_x + inner_w - toggle_w, row_y + 6.0)
    };
    controls.push(ControlBox {
        id,
        in_header: false,
        vis: RectL::new(vx, vy, toggle_w, 36.0),
        hit: RectL::new(vx, vy - 4.0, toggle_w, 44.0),
    });
}

fn push_stepper_controls(
    controls: &mut Vec<ControlBox>,
    dec_id: ControlId,
    inc_id: ControlId,
    narrow: bool,
    inner_x: f32,
    inner_w: f32,
    row_y: f32,
) -> RectL {
    let btn_w = 44.0;
    let val_w = 68.0;
    let gap = 6.0;
    let total_w = 2.0 * btn_w + val_w + 2.0 * gap;
    let (sx, sy) = if narrow {
        (inner_x, row_y + 34.0)
    } else {
        (inner_x + inner_w - total_w, row_y + 6.0)
    };
    controls.push(ControlBox {
        id: dec_id,
        in_header: false,
        vis: RectL::new(sx, sy, btn_w, 36.0),
        hit: RectL::new(sx, sy - 4.0, btn_w, 44.0),
    });
    let val_rect = RectL::new(sx + btn_w + gap, sy, val_w, 36.0);
    let ix = sx + btn_w + gap + val_w + gap;
    controls.push(ControlBox {
        id: inc_id,
        in_header: false,
        vis: RectL::new(ix, sy, btn_w, 36.0),
        hit: RectL::new(ix, sy - 4.0, btn_w, 44.0),
    });
    val_rect
}

fn fmt_offset_ms(ms: i64) -> String {
    if ms < 0 {
        format!("\u{2212}{} ms", ms.unsigned_abs())
    } else {
        format!("+{ms} ms")
    }
}

fn prefix_error(msg: &str) -> String {
    let trimmed = msg.trim();
    if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("error:") {
        trimmed.to_owned()
    } else {
        format!("Error: {trimmed}")
    }
}

fn paint_of(c: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(Color::from_rgba(c.0, c.1, c.2, c.3.clamp(0.0, 1.0)).unwrap_or(Color::WHITE));
    paint
}

fn fill_path(pm: &mut Pixmap, path: &tiny_skia::Path, c: Rgba, mask: Option<&Mask>) {
    let paint = paint_of(c);
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), mask);
}

fn fill_rrect(pm: &mut Pixmap, r: RectL, radius: f32, c: Rgba, mask: Option<&Mask>) {
    if let Some(path) = rrect(r.x, r.y, r.w, r.h, radius) {
        fill_path(pm, &path, c, mask);
    }
}

fn stroke_rrect(pm: &mut Pixmap, r: RectL, radius: f32, width: f32, c: Rgba, mask: Option<&Mask>) {
    let Some(path) = rrect(r.x, r.y, r.w, r.h, radius) else { return };
    let paint = paint_of(c);
    let stroke = Stroke { width: width.max(1.0), ..Stroke::default() };
    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), mask);
}

fn fill_rect_px(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, c: Rgba, mask: Option<&Mask>) {
    let Some(rect) = Rect::from_xywh(x, y, w, h) else { return };
    let paint = paint_of(c);
    pm.fill_rect(rect, &paint, Transform::identity(), mask);
}

fn draw_card(pm: &mut Pixmap, r: RectL, scale: f32, pal: &Palette, mask: Option<&Mask>) {
    fill_rrect(pm, r, 10.0 * scale, pal.elevated_rgba, mask);
    stroke_rrect(pm, r, 10.0 * scale, 1.0 * scale, pal.divider, mask);
}

fn draw_divider(
    pm: &mut Pixmap,
    layout: &Layout,
    y_logical: f32,
    scale: f32,
    dy: f32,
    pal: &Palette,
    mask: Option<&Mask>,
) {
    fill_rect_px(
        pm,
        (layout.inner_x * scale).round(),
        ((y_logical + dy) * scale).round(),
        (layout.inner_w * scale).round(),
        scale.max(1.0),
        pal.divider,
        mask,
    );
}

fn draw_focus_ring(
    pm: &mut Pixmap,
    vis_logical: RectL,
    scale: f32,
    dy: f32,
    base_radius: f32,
    accent: Rgba,
    mask: Option<&Mask>,
) {
    // Inflate outside the control edge so the 2px ring stays distinct even on active accent fills.
    let outer = vis_logical.inflate(2.5).scaled(scale, dy);
    stroke_rrect(pm, outer, (base_radius + 2.5) * scale, 2.0 * scale, accent, mask);
}

fn draw_close_icon(pm: &mut Pixmap, cx: f32, cy: f32, scale: f32, c: Rgba) {
    let d = 5.5 * scale;
    let mut pb = PathBuilder::new();
    pb.move_to(cx - d, cy - d);
    pb.line_to(cx + d, cy + d);
    pb.move_to(cx + d, cy - d);
    pb.line_to(cx - d, cy + d);
    let Some(path) = pb.finish() else { return };
    let paint = paint_of(c);
    let stroke = Stroke { width: (1.8 * scale).max(1.2), line_cap: LineCap::Round, ..Stroke::default() };
    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
}

fn draw_selector_arrows(
    pm: &mut Pixmap,
    cx: f32,
    cy: f32,
    scale: f32,
    c: Rgba,
    mask: Option<&Mask>,
) {
    let dx = 3.2 * scale;
    let dy_step = 2.6 * scale;
    let gap = 2.2 * scale;
    let mut pb = PathBuilder::new();
    // Up chevron
    pb.move_to(cx - dx, cy - gap);
    pb.line_to(cx, cy - gap - dy_step);
    pb.line_to(cx + dx, cy - gap);
    // Down chevron
    pb.move_to(cx - dx, cy + gap);
    pb.line_to(cx, cy + gap + dy_step);
    pb.line_to(cx + dx, cy + gap);
    let Some(path) = pb.finish() else { return };
    let paint = paint_of(c);
    let stroke = Stroke { width: (1.5 * scale).max(1.0), line_cap: LineCap::Round, ..Stroke::default() };
    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), mask);
}

fn read_clipboard_text() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            if let Some(s) = run_paste_cmd("wl-paste", &["--no-newline"]) {
                return Some(s);
            }
        }
        run_paste_cmd("xclip", &["-selection", "clipboard", "-o"])
            .or_else(|| run_paste_cmd("xsel", &["-ob"]))
    }
    #[cfg(target_os = "macos")]
    {
        run_paste_cmd("pbpaste", &[])
    }
    #[cfg(windows)]
    {
        run_paste_cmd("powershell", &["-NoProfile", "-NonInteractive", "-Command", "Get-Clipboard"])
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

fn run_paste_cmd(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8(out.stdout).ok()?;
    let clean: String = raw
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    if clean.is_empty() {
        None
    } else {
        Some(clean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn press_named(
        s: &mut Settings,
        named: NamedKey,
        mods: ModifiersState,
        win_h: f32,
    ) -> (Vec<UiAction>, bool, bool) {
        let code = match named {
            NamedKey::Tab => KeyCode::Tab,
            NamedKey::Escape => KeyCode::Escape,
            NamedKey::Enter => KeyCode::Enter,
            NamedKey::Space => KeyCode::Space,
            NamedKey::Backspace => KeyCode::Backspace,
            NamedKey::Delete => KeyCode::Delete,
            NamedKey::ArrowLeft => KeyCode::ArrowLeft,
            NamedKey::ArrowRight => KeyCode::ArrowRight,
            NamedKey::ArrowUp => KeyCode::ArrowUp,
            NamedKey::ArrowDown => KeyCode::ArrowDown,
            NamedKey::PageUp => KeyCode::PageUp,
            NamedKey::PageDown => KeyCode::PageDown,
            NamedKey::Home => KeyCode::Home,
            NamedKey::End => KeyCode::End,
            _ => KeyCode::F24,
        };
        s.handle_key(ElementState::Pressed, &Key::Named(named), PhysicalKey::Code(code), None, mods, win_h)
    }

    fn click_control(
        s: &mut Settings,
        id: ControlId,
        win_w: f32,
        win_h: f32,
    ) -> (Vec<UiAction>, bool, bool) {
        let layout = s.compute_layout(win_w);
        let ctrl = layout.find(id).expect("control exists in layout");
        if ctrl.in_header {
            let cx = ctrl.hit.x + ctrl.hit.w / 2.0;
            let cy = ctrl.hit.y + ctrl.hit.h / 2.0;
            return s.on_mouse_down(cx, cy, win_w, win_h);
        }
        // Scroll the target control into the visible body viewport before clicking its center.
        let body_h = win_h - HEADER_H;
        s.scroll_y = (ctrl.vis.y - body_h / 2.0).clamp(0.0, layout.max_scroll(win_h));
        let cx = ctrl.hit.x + ctrl.hit.w / 2.0;
        let cy = HEADER_H + (ctrl.hit.y - s.scroll_y) + ctrl.hit.h / 2.0;
        s.on_mouse_down(cx, cy, win_w, win_h)
    }

    #[test]
    fn responsive_narrow_and_wide_layouts_have_no_overflow_or_overlap() {
        let mut cfg = Config::default();
        cfg.ui.offset_ms = 250;
        cfg.lrc_dirs.push(PathBuf::from("/music/lyrics"));
        let mut s = Settings::new(&cfg, false);
        s.set_hotkey_errors(vec![(2, "Shortcut already in use".into())]);
        s.set_autostart_error(Some("Desktop entry write denied".into()));

        let narrow = s.compute_layout(380.0);
        let wide = s.compute_layout(520.0);
        assert!(narrow.narrow);
        assert!(!wide.narrow);
        assert!(narrow.content_h > wide.content_h, "stacked narrow layout grows vertically");

        let narrow_island = narrow.find(ControlId::ToggleIsland).unwrap();
        let wide_island = wide.find(ControlId::ToggleIsland).unwrap();
        assert_eq!(narrow_island.vis.x, narrow.inner_x, "narrow toggle stacks beneath label");
        assert!(wide_island.vis.x > wide.inner_x + 200.0, "wide toggle aligns to right column");

        for (w, layout) in [(MIN_W, s.compute_layout(MIN_W)), (380.0, narrow), (520.0, wide)] {
            for c in &layout.controls {
                assert!(c.vis.x >= 0.0 && c.vis.x + c.vis.w <= w, "vis overflow on {:?} at {w}", c.id);
                assert!(c.hit.x >= 0.0 && c.hit.x + c.hit.w <= w, "hit overflow on {:?} at {w}", c.id);
                assert!(c.vis.h >= 36.0, "visual height >= 36px on {:?}", c.id);
                assert!(c.hit.h >= 44.0 && c.hit.w >= 44.0, "44x44 tap target on {:?}", c.id);
            }
            let body_ctrls: Vec<&ControlBox> =
                layout.controls.iter().filter(|c| !c.in_header).collect();
            for i in 0..body_ctrls.len() {
                for j in (i + 1)..body_ctrls.len() {
                    assert!(
                        !body_ctrls[i].hit.intersects(&body_ctrls[j].hit),
                        "hit boxes {:?} and {:?} overlap at width {w}",
                        body_ctrls[i].id,
                        body_ctrls[j].id,
                    );
                }
            }
        }

        // Render smoke test across both themes and responsive widths.
        let mut text = Text::new();
        for (w, light) in [(380u32, false), (380u32, true), (520u32, false), (520u32, true)] {
            let mut pm = Pixmap::new(w, DEFAULT_H as u32).unwrap();
            s.render(&mut pm, &mut text, 1.0, light);
        }
    }

    #[test]
    fn keyboard_tab_traversal_and_escape_behavior() {
        let cfg = Config::default();
        let mut s = Settings::new(&cfg, false);
        let win_h = DEFAULT_H;
        let expected_order: Vec<ControlId> =
            s.compute_layout(DEFAULT_W).controls.iter().map(|c| c.id).collect();
        assert!(expected_order.len() >= 24);

        for &expected in &expected_order {
            let (actions, close, redraw) = press_named(&mut s, NamedKey::Tab, ModifiersState::empty(), win_h);
            assert!(actions.is_empty());
            assert!(!close);
            assert!(redraw);
            assert_eq!(s.focus, Some(expected));

            let layout = s.compute_layout(DEFAULT_W);
            let ctrl = layout.find(expected).unwrap();
            if !ctrl.in_header {
                let body_h = win_h - HEADER_H;
                assert!(ctrl.vis.y >= s.scroll_y, "{expected:?} scrolled above top");
                assert!(ctrl.vis.y + ctrl.vis.h <= s.scroll_y + body_h, "{expected:?} below viewport");
            }
        }
        assert!(s.scroll_y > 0.0, "Tab down to bottom controls auto-scrolls panel");

        // Next Tab wraps back to the first control; Shift+Tab steps back to the last control.
        press_named(&mut s, NamedKey::Tab, ModifiersState::empty(), win_h);
        assert_eq!(s.focus, Some(expected_order[0]));
        press_named(&mut s, NamedKey::Tab, ModifiersState::SHIFT, win_h);
        assert_eq!(s.focus, Some(*expected_order.last().unwrap()));

        // 1. Escape while recording a hotkey cancels recording without closing.
        s.focus = Some(ControlId::Hotkey(0));
        press_named(&mut s, NamedKey::Enter, ModifiersState::empty(), win_h);
        assert!(s.is_recording_hotkey());
        let (_, close, redraw) = press_named(&mut s, NamedKey::Escape, ModifiersState::empty(), win_h);
        assert!(!s.is_recording_hotkey());
        assert!(!close);
        assert!(redraw);

        // Capturing a valid shortcut emits SetHotkeys; Backspace disables it.
        press_named(&mut s, NamedKey::Space, ModifiersState::empty(), win_h);
        assert!(s.is_recording_hotkey());
        let (actions, close, _) = s.handle_key(
            ElementState::Pressed,
            &Key::Character("k".into()),
            PhysicalKey::Code(KeyCode::KeyK),
            Some("k"),
            ModifiersState::CONTROL | ModifiersState::ALT,
            win_h,
        );
        assert!(!close);
        assert!(!s.is_recording_hotkey());
        assert_eq!(actions.len(), 1);
        assert!(matches!(&actions[0], UiAction::SetHotkeys(hk) if hk.toggle_island == "Ctrl+Alt+K"));

        press_named(&mut s, NamedKey::Enter, ModifiersState::empty(), win_h);
        let (actions, _, _) = press_named(&mut s, NamedKey::Backspace, ModifiersState::empty(), win_h);
        assert!(matches!(&actions[0], UiAction::SetHotkeys(hk) if hk.toggle_island.is_empty()));

        // 2. Escape while folder input is focused clears focus first without closing.
        s.focus = Some(ControlId::LrcInput);
        let (_, close, redraw) = press_named(&mut s, NamedKey::Escape, ModifiersState::empty(), win_h);
        assert_eq!(s.focus, None);
        assert!(!close);
        assert!(redraw);

        // 3. Escape when idle requests window close.
        let (_, close, _) = press_named(&mut s, NamedKey::Escape, ModifiersState::empty(), win_h);
        assert!(close);
    }

    #[test]
    fn lrc_folder_validation_and_addition() {
        let cfg = Config::default();
        let mut s = Settings::new(&cfg, false);
        let win_h = DEFAULT_H;

        // Focus LrcInput, type a non-existent folder path, and submit with Enter.
        let _ = click_control(&mut s, ControlId::LrcInput, DEFAULT_W, win_h);
        let missing = "/nonexistent/lyrics-island-missing-folder-test";
        s.handle_key(
            ElementState::Pressed,
            &Key::Character(missing.into()),
            PhysicalKey::Code(KeyCode::Slash),
            Some(missing),
            ModifiersState::empty(),
            win_h,
        );
        let (actions, close, redraw) = press_named(&mut s, NamedKey::Enter, ModifiersState::empty(), win_h);
        assert!(actions.is_empty());
        assert!(!close);
        assert!(redraw);
        assert!(
            matches!(&s.lrc_feedback, Some(Feedback::Error(msg)) if msg.starts_with("Error: folder not found")),
            "expected text-first folder error, got {:?}",
            s.lrc_feedback
        );

        // Clear input and add a real temporary directory.
        press_named(&mut s, NamedKey::Delete, ModifiersState::empty(), win_h);
        let dir = std::env::temp_dir().join(format!("lyrics-island-lrc-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dir_str = dir.to_string_lossy().into_owned();

        s.handle_key(
            ElementState::Pressed,
            &Key::Character(dir_str.as_str().into()),
            PhysicalKey::Code(KeyCode::Slash),
            Some(&dir_str),
            ModifiersState::empty(),
            win_h,
        );
        let (actions, close, _) = click_control(&mut s, ControlId::AddLrcDir, DEFAULT_W, win_h);
        assert!(!close);
        assert_eq!(actions, vec![UiAction::SetLrcDirs(vec![dir.clone()])]);
        assert_eq!(s.lrc_feedback, Some(Feedback::Status("Status: Folder added".into())));

        // Remove the configured folder via its Remove button.
        let (actions, _, _) = click_control(&mut s, ControlId::RemoveLrcDir(0), DEFAULT_W, win_h);
        assert_eq!(actions, vec![UiAction::SetLrcDirs(Vec::new())]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn theme_steppers_and_reset_positions_emit_exact_actions() {
        let cfg = Config::default();
        let mut s = Settings::new(&cfg, false);
        s.set_players(vec![
            PlayerChoice { id: "spotify".into(), name: "Spotify".into() },
            PlayerChoice { id: "vlc".into(), name: "VLC".into() },
        ]);

        // Header drag vs Close button hit areas.
        assert!(s.is_drag_header(24.0, 24.0, DEFAULT_W));
        assert!(!s.is_drag_header(DEFAULT_W - 20.0, 24.0, DEFAULT_W));
        assert!(!s.is_drag_header(24.0, HEADER_H + 10.0, DEFAULT_W));

        // Theme segmented control.
        let (actions, close, _) = click_control(&mut s, ControlId::ThemeDark, DEFAULT_W, DEFAULT_H);
        assert!(!close);
        let mut expected_ui = cfg.ui.clone();
        expected_ui.theme = Theme::Dark;
        assert_eq!(actions, vec![UiAction::SetUiConfig(expected_ui.clone())]);

        // ArrowRight on focused ThemeDark advances to ThemeLight.
        let (actions, _, _) = press_named(&mut s, NamedKey::ArrowRight, ModifiersState::empty(), DEFAULT_H);
        expected_ui.theme = Theme::Light;
        assert_eq!(actions, vec![UiAction::SetUiConfig(expected_ui.clone())]);

        // Steppers: max compact width (+25), compact font (+1), desktop font (-2), offset (+250).
        let (actions, _, _) = click_control(&mut s, ControlId::MaxWidthInc, DEFAULT_W, DEFAULT_H);
        expected_ui.island_max_width = 875.0;
        assert_eq!(actions, vec![UiAction::SetUiConfig(expected_ui.clone())]);

        let (actions, _, _) = click_control(&mut s, ControlId::CompactFontInc, DEFAULT_W, DEFAULT_H);
        expected_ui.island_font_pt = 12.0;
        assert_eq!(actions, vec![UiAction::SetUiConfig(expected_ui.clone())]);

        let (actions, _, _) = click_control(&mut s, ControlId::DesktopFontDec, DEFAULT_W, DEFAULT_H);
        expected_ui.desktop_font_px = 32.0;
        assert_eq!(actions, vec![UiAction::SetUiConfig(expected_ui.clone())]);

        let (actions, _, _) = click_control(&mut s, ControlId::OffsetP250, DEFAULT_W, DEFAULT_H);
        expected_ui.offset_ms = 250;
        assert_eq!(actions, vec![UiAction::SetUiConfig(expected_ui.clone())]);

        let (actions, _, _) = click_control(&mut s, ControlId::OffsetReset, DEFAULT_W, DEFAULT_H);
        expected_ui.offset_ms = 0;
        assert_eq!(actions, vec![UiAction::SetUiConfig(expected_ui)]);

        // Window placement reset.
        let (actions, _, _) = click_control(&mut s, ControlId::ResetPositions, DEFAULT_W, DEFAULT_H);
        assert_eq!(actions, vec![UiAction::ResetPositions]);
        assert_eq!(s.positions_status.as_deref(), Some(POS_RESET_STATUS));

        // Media source cycling and autostart toggle.
        let (actions, _, _) = click_control(&mut s, ControlId::MediaSource, DEFAULT_W, DEFAULT_H);
        assert_eq!(actions, vec![UiAction::SelectPlayer(Some("spotify".into()))]);
        let (actions, _, _) = click_control(&mut s, ControlId::ToggleAutostart, DEFAULT_W, DEFAULT_H);
        assert_eq!(actions, vec![UiAction::SetAutostart(true)]);

        // Close button requests window close.
        let (actions, close, _) = click_control(&mut s, ControlId::Close, DEFAULT_W, DEFAULT_H);
        assert!(actions.is_empty());
        assert!(close);

        // Copy hygiene: zero em dashes in any constant string.
        for s_lit in HOTKEY_LABELS
            .into_iter()
            .chain([LRC_EMPTY_STATE, CAPTURE_PROMPT, POS_RESET_STATUS, lrc_placeholder()])
        {
            assert!(!s_lit.contains('\u{2014}'), "em dash forbidden in UI copy: {s_lit}");
        }
    }
}
