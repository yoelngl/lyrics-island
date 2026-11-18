//! The island, desktop-lyrics and settings windows: state, animation, input and per-OS presentation.

mod desktop;
mod island;
#[cfg(target_os = "macos")]
#[path = "present_macos.rs"]
mod present;
#[cfg(windows)]
#[path = "present_windows.rs"]
mod present;
#[cfg(target_os = "linux")]
#[path = "present_x11.rs"]
mod present;
mod settings;
mod text;

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tiny_skia::{Color, Path, PathBuilder, Pixmap};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId, WindowLevel};

use crate::config::Config;
use crate::model::{
    self, Command, Line, Lyrics, PlayerChoice, PlayerState, Status, Theme, UiAction, UiConfig, WindowKind,
};
use desktop::Desktop;
use island::{Bars, Hit, Island, EXPANDED_H, EXPANDED_W, PILL_MIN_W};
use settings::Settings;
use text::Text;

/// 120 Hz step for shape morphs, text swaps, fades and scrubbing; 60 Hz for steady playback visuals.
const FRAME_FAST: Duration = Duration::from_micros(8_333);
const FRAME_PLAYING: Duration = Duration::from_micros(16_667);
const COLLAPSE_DELAY: Duration = Duration::from_millis(1500);
/// Logical px the pointer must travel with the button down before a click becomes a window drag.
const DRAG_PX: f64 = 7.0;
const KEY_SEEK_MS: u64 = 5000;
/// Headroom around `EXPANDED_H` inside the fixed island canvas so spring settle never clips.
const CANVAS_PAD_H: f32 = 24.0;

pub struct Ui {
    cfg: UiConfig,
    mons: Vec<Mon>,
    primary: Mon,
    text: Text,
    isl_s: Surface,
    desk_s: Surface,
    set_s: Option<Surface>,
    isl: Island,
    desk: Desktop,
    settings: Settings,
    bars: Bars,
    /// Stationary compact-pill horizontal center and top edge in physical screen px.
    pill_anchor: (i32, i32),
    /// Pill horizontal center inside `isl_s` (physical px) and whether the card expands upward.
    canvas_cx: f32,
    expand_up: bool,
    system_light: bool,
    mods: ModifiersState,
    scrub_frac: Option<f32>,
    player: Option<PlayerState>,
    lyrics: Option<Arc<Lyrics>>,
    art: Option<Art>,
    /// "title · artist", shown when no line is active.
    fallback: String,
    /// (line index, active) the labels were last filled from; None forces a refresh.
    line_key: Option<(Option<usize>, bool)>,
    picked_idx: Option<usize>,
    /// Active line index and its sung part when karaoke applies.
    cur_line: Option<usize>,
    karaoke: Option<(Range<usize>, f32)>,
    elapsed_s: u64,
    start: Instant,
    next_frame: Instant,
}

impl Ui {
    pub fn new(event_loop: &ActiveEventLoop, full_cfg: &Config, autostart: bool) -> Ui {
        let cfg = &full_cfg.ui;
        let now = Instant::now();
        let (primary, mons) = Mon::discover(event_loop);
        let pill_anchor = resolve_pill_anchor(cfg.island_pos, &primary, &mons, cfg.island_font_pt);
        let (isl_size, isl_pos, canvas_cx, expand_up) =
            island_canvas_geometry(pill_anchor, cfg, &primary, &mons, primary.scale);
        let mut isl_s = Surface::new(event_loop, "Lyrics Island", isl_size, isl_pos, true, now);
        let (dsize, dpos) = desk_geometry(&primary, cfg, primary.scale);
        let mut desk_s = Surface::new(event_loop, "Lyrics Island Desktop Lyrics", dsize, dpos, true, now);
        desk_s.set_input_rect(None, primary.scale as f32, cfg.click_through);

        let isl = Island::new(now);
        let init_bounds = isl.bounds(isl_size, primary.scale as f32, canvas_cx, expand_up, now);
        isl_s.set_input_rect(Some(init_bounds), primary.scale as f32, cfg.click_through);

        let system_light = detect_system_light(&isl_s.window);
        let mut settings = Settings::new(full_cfg, autostart);
        settings.set_status_line("No active media player");

        Ui {
            cfg: cfg.clone(),
            mons,
            primary,
            text: Text::new(),
            isl_s,
            desk_s,
            set_s: None,
            isl,
            desk: Desktop::new(),
            settings,
            bars: Bars::new(now),
            pill_anchor,
            canvas_cx,
            expand_up,
            system_light,
            mods: ModifiersState::empty(),
            scrub_frac: None,
            player: None,
            lyrics: None,
            art: None,
            fallback: String::new(),
            line_key: None,
            picked_idx: None,
            cur_line: None,
            karaoke: None,
            elapsed_s: u64::MAX,
            start: now,
            next_frame: now,
        }
    }

    fn is_light(&self) -> bool {
        match self.cfg.theme {
            Theme::Light => true,
            Theme::Dark => false,
            Theme::System => self.system_light,
        }
    }

    pub fn open_settings(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if self.set_s.is_none() {
            let scale = self.primary.scale;
            let w = ((settings::DEFAULT_W as f64 * scale).round() as u32).min((self.primary.w as f64 * 0.92) as u32);
            let h = ((settings::DEFAULT_H as f64 * scale).round() as u32).min((self.primary.h as f64 * 0.9) as u32);
            let pos = (
                self.primary.x + (self.primary.w as i32 - w as i32) / 2,
                self.primary.y + (self.primary.h as i32 - h as i32) / 2,
            );
            self.set_s = Some(Surface::new(event_loop, "Lyrics Island Settings", (w, h), pos, false, now));
        }
        if let Some(s) = self.set_s.as_mut() {
            s.show_immediate();
            s.window.focus_window();
            s.redraw();
        }
    }

    pub fn sync_settings(&mut self, full_cfg: &Config, autostart: bool) {
        self.settings.sync(full_cfg, autostart);
        if let Some(s) = self.set_s.as_mut() {
            s.redraw();
        }
        self.apply_config(&full_cfg.ui);
    }

    pub fn set_players(&mut self, players: Vec<PlayerChoice>) {
        self.settings.set_players(players);
        if let Some(s) = self.set_s.as_mut() {
            s.redraw();
        }
    }

    pub fn set_hotkey_errors(&mut self, errs: Vec<(usize, String)>) {
        self.settings.set_hotkey_errors(errs);
        if let Some(s) = self.set_s.as_mut() {
            s.redraw();
        }
    }

    pub fn set_autostart_error(&mut self, err: Option<String>) {
        self.settings.set_autostart_error(err);
        if let Some(s) = self.set_s.as_mut() {
            s.redraw();
        }
    }

    pub fn apply_config(&mut self, cfg: &UiConfig) {
        if self.cfg == *cfg {
            return;
        }
        let now = Instant::now();
        let old = std::mem::replace(&mut self.cfg, cfg.clone());
        if old.click_through != cfg.click_through {
            let dscale = self.desk_s.window.scale_factor() as f32;
            self.desk_s.set_input_rect(None, dscale, cfg.click_through);
            self.sync_island_input_rect(now);
            if cfg.click_through {
                self.collapse(now);
            }
        }
        if old.island_pos != cfg.island_pos
            || old.island_max_width != cfg.island_max_width
            || old.island_font_pt != cfg.island_font_pt
        {
            self.pill_anchor = resolve_pill_anchor(cfg.island_pos, &self.primary, &self.mons, cfg.island_font_pt);
            self.place_island_canvas(now);
            self.fit_pill(now, old.island_pos == cfg.island_pos);
        }
        if old.desktop_pos != cfg.desktop_pos || old.desktop_font_px != cfg.desktop_font_px {
            self.place_desktop();
        }
        self.line_key = None;
        self.isl_s.redraw();
        self.desk_s.redraw();
        if let Some(s) = self.set_s.as_mut() {
            s.redraw();
        }
    }

    pub fn set_player(&mut self, state: Option<PlayerState>) {
        if let Some(p) = &state {
            let (title, artist) = (&p.track.title, &p.track.artist);
            if self.isl.title.text() != title || self.isl.artist.text() != artist {
                self.isl.title.set(title);
                self.isl.artist.set(artist);
                self.fallback = if artist.is_empty() { title.clone() } else { format!("{title} · {artist}") };
                self.line_key = None;
            }
            let len = p.track.length_ms;
            self.isl.total.set(&if len > 0 { fmt_time(len) } else { "--:--".to_owned() });
            self.elapsed_s = u64::MAX;
            let prefix = if p.status == Status::Playing { "Playing" } else { "Paused" };
            self.settings.set_status_line(&format!("{prefix}: {}", self.fallback));
        } else {
            self.settings.set_status_line("No active media player");
        }
        self.player = state;
        self.isl_s.redraw();
        self.desk_s.redraw();
        if let Some(s) = self.set_s.as_mut() {
            s.redraw();
        }
    }

    pub fn set_lyrics(&mut self, lyrics: Option<Arc<Lyrics>>) {
        self.isl.credit.set(&lyrics.as_ref().map_or(String::new(), |l| format!("Lyrics · {}", l.source)));
        self.lyrics = lyrics;
        self.line_key = None;
        self.isl_s.redraw();
        self.desk_s.redraw();
    }

    pub fn set_art(&mut self, art: Option<Arc<model::Rgba>>) {
        self.art = art.map(Art::new);
        self.isl_s.redraw();
    }

    pub fn is_recording_hotkey(&self) -> bool {
        self.set_s.as_ref().is_some_and(|s| s.shown) && self.settings.is_recording_hotkey()
    }

    pub fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) -> Vec<UiAction> {
        let mut out = Vec::new();
        let kind = if id == self.isl_s.window.id() {
            WindowKind::Island
        } else if id == self.desk_s.window.id() {
            WindowKind::Desktop
        } else if self.set_s.as_ref().is_some_and(|s| s.window.id() == id) {
            WindowKind::Settings
        } else {
            return out;
        };
        if kind == WindowKind::Settings {
            return self.settings_window_event(event);
        }

        let island = kind == WindowKind::Island;
        let now = Instant::now();
        match event {
            WindowEvent::RedrawRequested => self.render(kind, now),
            WindowEvent::Resized(_) => {
                let s = self.surface(kind);
                s.prev_bounds = None;
                s.redraw();
            }
            WindowEvent::ThemeChanged(theme) => {
                self.system_light = matches!(theme, winit::window::Theme::Light);
                self.isl_s.redraw();
                if let Some(s) = self.set_s.as_mut() {
                    s.redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::ScaleFactorChanged { .. } => {
                (self.primary, self.mons) = Mon::discover(event_loop);
                if island {
                    self.place_island_canvas(now);
                    self.fit_pill(now, false);
                } else {
                    self.place_desktop();
                }
                self.surface(kind).redraw();
            }
            WindowEvent::Moved(p) => {
                let s = self.surface(kind);
                if s.dragging && (p.x, p.y) != s.pos {
                    s.dragged_to = Some((p.x, p.y));
                }
            }
            WindowEvent::Focused(false) if island => {
                self.scrub_frac = None;
                self.collapse(now);
            }
            WindowEvent::KeyboardInput { event, .. } if island && event.state == ElementState::Pressed => {
                if let Some(a) = self.on_island_key(&event.logical_key, now) {
                    out.push(a);
                }
            }
            WindowEvent::CursorEntered { .. } => {
                self.end_drag(event_loop, kind, false, now, &mut out);
                if island {
                    self.isl.left_at = None;
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.end_drag(event_loop, kind, false, now, &mut out);
                if island {
                    self.isl.left_at = Some(now);
                    self.set_hover(None);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.end_drag(event_loop, kind, false, now, &mut out);
                self.surface(kind).cursor = position;
                if island && self.scrub_frac.is_some() {
                    self.scrub_frac = Some(self.progress_frac_under());
                    self.isl_s.redraw();
                    return out;
                }
                let s = self.surface(kind);
                if let Some(p0) = s.press {
                    if (position.x - p0.x).hypot(position.y - p0.y) > DRAG_PX * s.window.scale_factor() {
                        s.press = None;
                        s.drag_origin_pos = s.pos;
                        s.dragged_to = None;
                        s.dragging = s.window.drag_window().is_ok();
                        self.isl.pressed = None;
                    }
                }
                if island {
                    let h = self.hit_under();
                    self.set_hover(h);
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.end_drag(event_loop, kind, true, now, &mut out);
                match state {
                    ElementState::Pressed => {
                        if island {
                            if !self.point_in_island_shape(now) {
                                return out;
                            }
                            if self.isl.expanded {
                                let hit = self.hit_under();
                                if hit == Some(Hit::Progress) {
                                    self.isl_s.press = None;
                                    self.isl.pressed = Some(Hit::Progress);
                                    self.scrub_frac = Some(self.progress_frac_under());
                                    self.isl_s.redraw();
                                    return out;
                                }
                                self.isl_s.press = Some(self.isl_s.cursor);
                                self.isl.pressed = hit;
                                self.isl_s.redraw();
                                return out;
                            }
                        }
                        let s = self.surface(kind);
                        s.press = Some(s.cursor);
                    }
                    ElementState::Released => {
                        if island {
                            if let Some(frac) = self.scrub_frac.take() {
                                self.isl.pressed = None;
                                if let Some(len) = self.player.as_ref().map(|p| p.track.length_ms).filter(|&l| l > 0) {
                                    let ms = (frac as f64 * len as f64).round() as u64;
                                    out.push(UiAction::Player(Command::Seek(ms.min(len))));
                                }
                                self.isl_s.redraw();
                                return out;
                            }
                        }
                        if self.surface(kind).press.take().is_some() && island {
                            out.extend(self.click(now));
                        }
                        if self.isl.pressed.take().is_some() {
                            self.isl_s.redraw();
                        }
                    }
                }
            }
            _ => {}
        }
        out
    }

    fn settings_window_event(&mut self, event: WindowEvent) -> Vec<UiAction> {
        let mut out = Vec::new();
        let now = Instant::now();
        match event {
            WindowEvent::RedrawRequested => self.render(WindowKind::Settings, now),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(s) = self.set_s.as_mut() {
                    s.prev_bounds = None;
                    s.redraw();
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                self.system_light = matches!(theme, winit::window::Theme::Light);
                self.isl_s.redraw();
                if let Some(s) = self.set_s.as_mut() {
                    s.redraw();
                }
            }
            WindowEvent::CloseRequested => self.close_settings(),
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let Some(s) = self.set_s.as_mut() else { return out };
                s.cursor = position;
                let scale = s.window.scale_factor();
                if let Some(p0) = s.press {
                    if (position.x - p0.x).hypot(position.y - p0.y) > DRAG_PX * scale {
                        s.press = None;
                        let _ = s.window.drag_window();
                        return out;
                    }
                }
                let sz = s.window.inner_size();
                if self.settings.on_cursor_move(
                    (position.x / scale) as f32,
                    (position.y / scale) as f32,
                    (sz.width as f64 / scale) as f32,
                    (sz.height as f64 / scale) as f32,
                ) {
                    s.redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(s) = self.set_s.as_mut() else { return out };
                let scale = s.window.scale_factor();
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 36.0,
                    MouseScrollDelta::PixelDelta(p) => (p.y / scale) as f32,
                };
                let win_h = (s.window.inner_size().height as f64 / scale) as f32;
                if self.settings.on_mouse_wheel(dy, win_h) {
                    s.redraw();
                }
            }
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                let Some(s) = self.set_s.as_mut() else { return out };
                let scale = s.window.scale_factor();
                let sz = s.window.inner_size();
                let (lx, ly) = ((s.cursor.x / scale) as f32, (s.cursor.y / scale) as f32);
                let (ww, wh) = ((sz.width as f64 / scale) as f32, (sz.height as f64 / scale) as f32);
                match state {
                    ElementState::Pressed => {
                        if self.settings.is_drag_header(lx, ly, ww) {
                            s.press = Some(s.cursor);
                        } else {
                            s.press = None;
                            let (acts, close, dirty) = self.settings.on_mouse_down(lx, ly, ww, wh);
                            out.extend(acts);
                            if close {
                                self.close_settings();
                            } else if dirty {
                                if let Some(s) = self.set_s.as_mut() {
                                    s.redraw();
                                }
                            }
                        }
                    }
                    ElementState::Released => {
                        s.press = None;
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let Some(s) = self.set_s.as_ref() else { return out };
                let wh = (s.window.inner_size().height as f64 / s.window.scale_factor()) as f32;
                let (acts, close, dirty) = self.settings.on_key(&event, self.mods, wh);
                out.extend(acts);
                if close {
                    self.close_settings();
                } else if dirty {
                    if let Some(s) = self.set_s.as_mut() {
                        s.redraw();
                    }
                }
            }
            _ => {}
        }
        out
    }

    fn close_settings(&mut self) {
        if let Some(s) = self.set_s.as_mut() {
            s.hide_immediate();
        }
    }

    pub fn update(&mut self) -> Option<Instant> {
        let now = Instant::now();
        let active = self.player.as_ref().is_some_and(|p| p.status != Status::Stopped);
        let playing = self.player.as_ref().is_some_and(|p| p.status == Status::Playing);
        let want_island = active && self.cfg.island_visible;
        if !want_island {
            self.collapse(now);
        }
        self.isl_s.show(want_island, now);
        self.desk_s.show(active && self.cfg.desktop_visible, now);

        if let Some(p) = &self.player {
            let pos = p.position_now();
            if pos / 1000 != self.elapsed_s {
                self.elapsed_s = pos / 1000;
                self.isl.elapsed.set(&fmt_time(pos));
                self.isl_s.dirty |= self.isl.expanded;
            }
            self.refresh_lines(pos as i64 + self.cfg.offset_ms, now);
        }
        if self.isl.line.step(now) {
            self.fit_pill(now, true);
        }
        self.isl.cur.step(now);
        self.bars.step(now, playing && self.isl_s.shown);

        let mut wake = None;
        if let (true, Some(left)) = (self.isl.expanded, self.isl.left_at) {
            if self.scrub_frac.is_none() {
                let at = left + COLLAPSE_DELAY;
                if now >= at {
                    self.collapse(now);
                } else {
                    wake = Some(at);
                }
            }
        }
        self.sync_island_input_rect(now);

        let fast_anim = (self.isl_s.shown
            && (self.isl.transitioning(now) || !self.isl_s.fade.done(now) || self.scrub_frac.is_some()))
            || (self.desk_s.shown && !self.desk_s.fade.done(now));
        let isl_live = self.isl_s.shown && (fast_anim || playing || self.bars.animating(now));
        let desk_live = self.desk_s.shown && (!self.desk_s.fade.done(now) || (playing && self.karaoke.is_some()));
        let frame_dt = if fast_anim { FRAME_FAST } else { FRAME_PLAYING };

        if (isl_live || desk_live) && now >= self.next_frame {
            self.next_frame = now + frame_dt;
            self.isl_s.dirty |= isl_live;
            self.desk_s.dirty |= desk_live;
        }
        for s in [&mut self.isl_s, &mut self.desk_s] {
            if !s.want && s.shown && s.fade.done(now) {
                s.window.set_visible(false);
                s.shown = false;
            }
            if s.shown && s.dirty {
                s.window.request_redraw();
            }
        }
        if let Some(s) = self.set_s.as_mut() {
            if s.shown && s.dirty {
                s.window.request_redraw();
            }
        }
        if isl_live || desk_live {
            Some(wake.map_or(self.next_frame, |w: Instant| w.min(self.next_frame)))
        } else {
            wake
        }
    }

    fn surface(&mut self, kind: WindowKind) -> &mut Surface {
        match kind {
            WindowKind::Island => &mut self.isl_s,
            WindowKind::Desktop => &mut self.desk_s,
            WindowKind::Settings => self.set_s.as_mut().expect("settings surface"),
        }
    }

    fn refresh_lines(&mut self, t: i64, now: Instant) {
        let lyrics = self.lyrics.clone();
        let lines: &[Line] = lyrics.as_deref().map_or(&[][..], |l| l.lines.as_slice());
        let (idx, active) = pick(lines, t);
        self.picked_idx = idx;
        self.cur_line = idx.filter(|_| active);
        self.karaoke = self
            .cur_line
            .map(|i| &lines[i])
            .filter(|l| self.cfg.karaoke && !l.words.is_empty())
            .map(|l| sung(l, t));
        if self.line_key == Some((idx, active)) {
            return;
        }
        self.line_key = Some((idx, active));

        let fb = self.fallback.as_str();
        let line = idx.map(|i| lines[i].text.as_str());
        let compact = if active { line.unwrap_or(fb) } else { fb };
        let cur = if active { compact } else { line.filter(|s| !s.is_empty()).unwrap_or(fb) };
        let prev = match idx {
            Some(i) if i > 0 => lines[i - 1].text.as_str(),
            _ => "",
        };
        let next = match idx {
            Some(i) => lines.get(i + 1),
            None => lines.first(),
        }
        .map_or("", |l| l.text.as_str());

        let animate = self.isl_s.shown && self.isl_s.want;
        let applied = self.isl.line.set(compact, animate && !self.isl.expanded, now);
        self.isl.cur.set(cur, animate && self.isl.expanded, now);
        self.isl.prev.set(prev);
        self.isl.next.set(next);
        self.desk.cur.set(compact);
        self.desk.next.set(next);
        self.isl_s.redraw();
        self.desk_s.redraw();
        if applied {
            self.fit_pill(now, animate);
        }
    }

    fn fit_pill(&mut self, now: Instant, animate: bool) {
        let mon = monitor_at(self.pill_anchor, &self.primary, &self.mons);
        let scale = self.isl_s.window.scale_factor();
        let canvas_w_l = (self.isl_s.size.0 as f64 / scale) as f32;
        let screen_max = (mon.w as f64 / mon.scale * 0.85) as f32;
        let max = self.cfg.island_max_width.min(screen_max).min(canvas_w_l);
        self.isl.fit_pill(&mut self.text, scale as f32, self.cfg.island_font_pt, max, now, animate);
        self.sync_island_input_rect(now);
    }

    fn place_island_canvas(&mut self, now: Instant) {
        let scale = self.isl_s.window.scale_factor();
        let (size, pos, cx, expand_up) =
            island_canvas_geometry(self.pill_anchor, &self.cfg, &self.primary, &self.mons, scale);
        self.canvas_cx = cx;
        self.expand_up = expand_up;
        self.isl_s.place(size, pos);
        self.sync_island_input_rect(now);
    }

    fn sync_island_input_rect(&mut self, now: Instant) {
        let scale = self.isl_s.window.scale_factor() as f32;
        let bounds = self.isl.bounds(self.isl_s.size, scale, self.canvas_cx, self.expand_up, now);
        self.isl_s.set_input_rect(Some(bounds), scale, self.cfg.click_through);
    }

    fn place_desktop(&mut self) {
        let (size, pos) = desk_geometry(&self.primary, &self.cfg, self.desk_s.window.scale_factor());
        self.desk_s.place(size, pos);
    }

    fn expand(&mut self, now: Instant) {
        if self.cfg.click_through || self.isl.expanded {
            return;
        }
        self.isl.expand(now);
        self.sync_island_input_rect(now);
        self.isl_s.window.focus_window();
        self.isl_s.redraw();
    }

    fn collapse(&mut self, now: Instant) {
        if self.isl.expanded {
            self.scrub_frac = None;
            self.isl.collapse(now);
            self.fit_pill(now, true);
            self.isl_s.redraw();
        }
    }

    fn prev_line_target_ms(&self) -> Option<u64> {
        let lines = self.lyrics.as_ref()?.lines.as_slice();
        let i = self.picked_idx?;
        let target = lines.get(i.checked_sub(1)?)?.start_ms as i64 - self.cfg.offset_ms;
        Some(target.max(0) as u64)
    }

    fn next_line_target_ms(&self) -> Option<u64> {
        let lines = self.lyrics.as_ref()?.lines.as_slice();
        let line = match self.picked_idx {
            Some(i) => lines.get(i + 1)?,
            None => lines.first()?,
        };
        let target = line.start_ms as i64 - self.cfg.offset_ms;
        Some(target.max(0) as u64)
    }

    fn line_nav_flags(&self) -> (bool, bool, bool) {
        let has_prev = self.prev_line_target_ms().is_some();
        let has_next = self.next_line_target_ms().is_some();
        let has_len = self.player.as_ref().is_some_and(|p| p.track.length_ms > 0);
        (has_prev, has_next, has_len)
    }

    fn click(&mut self, now: Instant) -> Option<UiAction> {
        if !self.isl.expanded {
            self.expand(now);
            return None;
        }
        match self.hit_under() {
            Some(h) if self.isl.pressed == Some(h) => self.activate_hit(h),
            Some(_) => None,
            None => {
                self.collapse(now);
                None
            }
        }
    }

    fn activate_hit(&self, hit: Hit) -> Option<UiAction> {
        match hit {
            Hit::Button(cmd) => Some(UiAction::Player(cmd)),
            Hit::PrevLine => self.prev_line_target_ms().map(|ms| UiAction::Player(Command::Seek(ms))),
            Hit::NextLine => self.next_line_target_ms().map(|ms| UiAction::Player(Command::Seek(ms))),
            Hit::Progress => None,
        }
    }

    fn on_island_key(&mut self, key: &Key, now: Instant) -> Option<UiAction> {
        if !self.isl.expanded {
            return None;
        }
        let (has_prev, has_next, has_len) = self.line_nav_flags();
        match key {
            Key::Named(NamedKey::Escape) => {
                self.collapse(now);
                None
            }
            Key::Named(NamedKey::Tab) => {
                self.isl.move_focus(self.mods.shift_key(), has_prev, has_next, has_len);
                self.isl_s.redraw();
                None
            }
            Key::Named(NamedKey::Enter) => self.isl.focus.and_then(|f| self.activate_hit(f)),
            Key::Named(NamedKey::Space) => match self.isl.focus {
                Some(f) if f != Hit::Progress => self.activate_hit(f),
                _ => Some(UiAction::Player(Command::PlayPause)),
            },
            Key::Named(NamedKey::ArrowLeft) => {
                let pos = self.player.as_ref()?.position_now();
                Some(UiAction::Player(Command::Seek(pos.saturating_sub(KEY_SEEK_MS))))
            }
            Key::Named(NamedKey::ArrowRight) => {
                let p = self.player.as_ref()?;
                let pos = p.position_now() + KEY_SEEK_MS;
                let clamped = if p.track.length_ms > 0 { pos.min(p.track.length_ms) } else { pos };
                Some(UiAction::Player(Command::Seek(clamped)))
            }
            _ => None,
        }
    }

    fn point_in_island_shape(&self, now: Instant) -> bool {
        let s = self.isl_s.window.scale_factor() as f32;
        let sz = self.isl_s.window.inner_size();
        let (ox, oy, w, h) = self.isl.shape_rect((sz.width, sz.height), s, self.canvas_cx, self.expand_up, now);
        let c = self.isl_s.cursor;
        (ox..=ox + w).contains(&(c.x as f32)) && (oy..=oy + h).contains(&(c.y as f32))
    }

    fn cursor_logical_in_expanded(&self) -> (f32, f32) {
        let s = self.isl_s.window.scale_factor() as f32;
        let sz = self.isl_s.window.inner_size();
        let (eox, eoy) = self.isl.expanded_origin((sz.width, sz.height), s, self.canvas_cx, self.expand_up);
        let c = self.isl_s.cursor;
        ((c.x as f32 - eox) / s, (c.y as f32 - eoy) / s)
    }

    fn hit_under(&self) -> Option<Hit> {
        let (x, y) = self.cursor_logical_in_expanded();
        let (has_prev, has_next, has_len) = self.line_nav_flags();
        self.isl.hit_at(x, y, has_prev, has_next, has_len)
    }

    fn progress_frac_under(&self) -> f32 {
        let (x, _) = self.cursor_logical_in_expanded();
        self.isl.progress_frac(x)
    }

    fn set_hover(&mut self, h: Option<Hit>) {
        if self.isl.hover != h {
            self.isl.hover = h;
            self.isl_s.redraw();
        }
    }

    /// Applies the drag delta `(dx, dy)` to the stationary compact-pill anchor so dragging
    /// while expanded or collapsed never shifts the small lyrics relative to where you dropped it.
    fn end_drag(
        &mut self,
        event_loop: &ActiveEventLoop,
        kind: WindowKind,
        force: bool,
        now: Instant,
        out: &mut Vec<UiAction>,
    ) {
        let s = self.surface(kind);
        if !s.dragging || !(force || s.dragged_to.is_some()) {
            return;
        }
        s.dragging = false;
        let origin = s.drag_origin_pos;
        let Some(p) = s.dragged_to.take() else { return };
        s.pos = p;
        if kind == WindowKind::Island {
            (self.primary, self.mons) = Mon::discover(event_loop);
            let raw = (self.pill_anchor.0 + (p.0 - origin.0), self.pill_anchor.1 + (p.1 - origin.1));
            let next_anchor = resolve_pill_anchor(Some(raw), &self.primary, &self.mons, self.cfg.island_font_pt);
            self.pill_anchor = next_anchor;
            self.place_island_canvas(now);
            self.isl_s.prev_bounds = None;
            self.isl_s.redraw();
            out.push(UiAction::Moved(WindowKind::Island, next_anchor));
        } else {
            out.push(UiAction::Moved(kind, p));
        }
    }

    fn render(&mut self, kind: WindowKind, now: Instant) {
        let light = self.is_light();
        let (has_prev, has_next, _) = self.line_nav_flags();
        let Ui {
            text,
            isl_s,
            desk_s,
            set_s,
            isl,
            desk,
            settings,
            art,
            bars,
            player,
            lyrics,
            cfg,
            karaoke,
            cur_line,
            canvas_cx,
            expand_up,
            start,
            scrub_frac,
            ..
        } = self;
        let s = match kind {
            WindowKind::Island => isl_s,
            WindowKind::Desktop => desk_s,
            WindowKind::Settings => {
                let Some(s) = set_s.as_mut() else { return };
                s
            }
        };
        s.dirty = false;
        let size = s.window.inner_size();
        if !s.shown || size.width == 0 || size.height == 0 {
            return;
        }
        let resized = s.pixmap.as_ref().is_none_or(|p| p.width() != size.width || p.height() != size.height);
        if resized {
            s.pixmap = Pixmap::new(size.width, size.height);
            s.prev_bounds = None;
        }
        let Some(pm) = s.pixmap.as_mut() else { return };
        let scale = s.window.scale_factor() as f32;
        let line = (*cur_line).and_then(|i| lyrics.as_ref()?.lines.get(i)).map(|l| l.text.as_str());
        let kara = match (karaoke, line) {
            (Some((r, f)), Some(l)) => Some((l, r.clone(), *f)),
            _ => None,
        };

        let (damage, input_rect) = match kind {
            WindowKind::Island => {
                let cur_b = isl.bounds((size.width, size.height), scale, *canvas_cx, *expand_up, now);
                let cur_u = (cur_b.0.max(0) as u32, cur_b.1.max(0) as u32, cur_b.2, cur_b.3);
                let dmg = match s.prev_bounds {
                    Some(prev) => Some(union_rect(prev, cur_u)),
                    None => {
                        pm.fill(Color::TRANSPARENT);
                        None
                    }
                };
                if let Some((dx, dy, dw, dh)) = dmg {
                    clear_rect(pm, dx, dy, dw, dh);
                }
                s.prev_bounds = Some(cur_u);

                let small_px = (island::compact_art_size(cfg.island_font_pt) * scale).round() as u32;
                if let Some(a) = art.as_mut() {
                    a.prepare(small_px, (56.0 * scale).round() as u32);
                }
                let p = player.as_ref();
                let ctx = island::Ctx {
                    now,
                    scale,
                    light,
                    font_pt: cfg.island_font_pt,
                    center_x: *canvas_cx,
                    expand_up: *expand_up,
                    clock: now.duration_since(*start).as_secs_f32(),
                    playing: p.is_some_and(|p| p.status == Status::Playing),
                    pos_ms: p.map_or(0, |p| p.position_now()),
                    len_ms: p.map_or(0, |p| p.track.length_ms),
                    scrub_frac: *scrub_frac,
                    has_prev_line: has_prev,
                    has_next_line: has_next,
                    karaoke: kara,
                    bars: bars.values(now),
                    art_small: art.as_ref().and_then(|a| a.small.as_ref()),
                    art_large: art.as_ref().and_then(|a| a.large.as_ref()),
                    gradient: art.as_ref().and_then(|a| a.gradient),
                };
                isl.render(pm, text, &ctx);
                // Content is laid out at its final size while the shape morphs, so it can spill past the shape;
                // anything outside cur_u would reach the screen via a damage blit and never be overwritten.
                let (pw, ph) = (pm.width(), pm.height());
                let (x, y, w, h) = cur_u;
                clear_rect(pm, 0, 0, pw, y);
                clear_rect(pm, 0, y + h, pw, ph);
                clear_rect(pm, 0, y, x, h);
                clear_rect(pm, x + w, y, pw, h);
                // No prev_bounds (show, resize, drag end moving the pill in the canvas): blit everything.
                (dmg, Some(cur_b))
            }
            WindowKind::Desktop => {
                pm.fill(Color::TRANSPARENT);
                desk.render(pm, text, scale, cfg.desktop_font_px, !cfg.click_through, kara);
                (None, None)
            }
            WindowKind::Settings => {
                pm.fill(Color::TRANSPARENT);
                settings.render(pm, text, scale, light);
                (None, None)
            }
        };

        let a = s.fade.value(now);
        if a < 0.999 {
            if let Some((dx, dy, dw, dh)) = damage {
                scale_rect_alpha(pm, dx, dy, dw, dh, a);
            } else {
                for b in pm.data_mut() {
                    *b = (*b as f32 * a) as u8;
                }
            }
        }
        s.presenter.present(&s.window, pm, damage);
        if let Some(cur_b) = input_rect {
            s.set_input_rect(Some(cur_b), scale, cfg.click_through);
        }
    }
}

fn union_rect(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let x0 = a.0.min(b.0);
    let y0 = a.1.min(b.1);
    let x1 = (a.0 + a.2).max(b.0 + b.2);
    let y1 = (a.1 + a.3).max(b.1 + b.3);
    (x0, y0, x1 - x0, y1 - y0)
}

fn clear_rect(pm: &mut Pixmap, x: u32, y: u32, w: u32, h: u32) {
    let (pw, ph) = (pm.width() as usize, pm.height() as usize);
    let x0 = (x as usize).min(pw);
    let y0 = (y as usize).min(ph);
    let x1 = (x0 + w as usize).min(pw);
    let y1 = (y0 + h as usize).min(ph);
    let data = pm.data_mut();
    for row in y0..y1 {
        data[(row * pw + x0) * 4..(row * pw + x1) * 4].fill(0);
    }
}

fn scale_rect_alpha(pm: &mut Pixmap, x: u32, y: u32, w: u32, h: u32, a: f32) {
    let (pw, ph) = (pm.width() as usize, pm.height() as usize);
    let x0 = (x as usize).min(pw);
    let y0 = (y as usize).min(ph);
    let x1 = (x0 + w as usize).min(pw);
    let y1 = (y0 + h as usize).min(ph);
    let data = pm.data_mut();
    for row in y0..y1 {
        for b in &mut data[(row * pw + x0) * 4..(row * pw + x1) * 4] {
            *b = (*b as f32 * a) as u8;
        }
    }
}

/// One window plus what's needed to draw, show and drag it.
struct Surface {
    window: Window,
    presenter: present::Presenter,
    pixmap: Option<Pixmap>,
    prev_bounds: Option<(u32, u32, u32, u32)>,
    overlay: bool,
    shown: bool,
    want: bool,
    fade: Anim,
    dirty: bool,
    cursor: PhysicalPosition<f64>,
    press: Option<PhysicalPosition<f64>>,
    dragging: bool,
    drag_origin_pos: (i32, i32),
    dragged_to: Option<(i32, i32)>,
    size: (u32, u32),
    pos: (i32, i32),
}

impl Surface {
    fn new(el: &ActiveEventLoop, title: &str, size: (u32, u32), pos: (i32, i32), overlay: bool, now: Instant) -> Self {
        let mut attrs = Window::default_attributes()
            .with_title(title)
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(!overlay)
            .with_visible(false)
            .with_active(!overlay)
            .with_window_level(if overlay { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal })
            .with_inner_size(PhysicalSize::new(size.0, size.1))
            .with_position(PhysicalPosition::new(pos.0, pos.1));
        if !overlay {
            attrs = attrs.with_min_inner_size(winit::dpi::LogicalSize::new(settings::MIN_W, settings::MIN_H));
        }
        #[cfg(windows)]
        if overlay {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs = attrs.with_skip_taskbar(true);
        }
        #[cfg(target_os = "linux")]
        if overlay {
            use winit::platform::x11::{WindowAttributesExtX11, WindowType};
            attrs = attrs.with_x11_window_type(vec![WindowType::Utility]);
        }
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs = attrs.with_has_shadow(!overlay);
        }
        let window = el.create_window(attrs).expect("failed to create window");
        let presenter = present::Presenter::new(&window);
        Self {
            window,
            presenter,
            pixmap: None,
            prev_bounds: None,
            overlay,
            shown: false,
            want: false,
            fade: Anim::new(if overlay { 0.0 } else { 1.0 }, now),
            dirty: true,
            cursor: PhysicalPosition::new(0.0, 0.0),
            press: None,
            dragging: false,
            drag_origin_pos: pos,
            dragged_to: None,
            size,
            pos,
        }
    }

    fn set_input_rect(&mut self, rect: Option<(i32, i32, u32, u32)>, scale: f32, click_through: bool) {
        let _ = self.window.set_cursor_hittest(!click_through);
        #[cfg(target_os = "linux")]
        self.presenter.set_input_rect(rect, scale, click_through);
        #[cfg(not(target_os = "linux"))]
        let _ = (rect, scale);
    }

    fn redraw(&mut self) {
        self.dirty = true;
        if self.shown {
            self.window.request_redraw();
        }
    }

    fn show(&mut self, want: bool, now: Instant) {
        if want == self.want {
            return;
        }
        self.want = want;
        if want && !self.shown {
            #[cfg(target_os = "linux")]
            self.presenter.prepare_show(self.overlay);
            self.prev_bounds = None;
            self.window.set_visible(true);
            self.shown = true;
        }
        self.fade.set(if want { 1.0 } else { 0.0 }, 220, 0, ease_out_quad, now);
        self.redraw();
    }

    fn show_immediate(&mut self) {
        self.want = true;
        self.fade.jump(1.0);
        if !self.shown {
            #[cfg(target_os = "linux")]
            self.presenter.prepare_show(self.overlay);
            self.prev_bounds = None;
            self.window.set_visible(true);
            self.shown = true;
        }
        self.redraw();
    }

    fn hide_immediate(&mut self) {
        self.want = false;
        self.shown = false;
        self.press = None;
        self.window.set_visible(false);
    }

    fn place(&mut self, size: (u32, u32), pos: (i32, i32)) {
        if size != self.size {
            self.size = size;
            self.prev_bounds = None;
            let _ = self.window.request_inner_size(PhysicalSize::new(size.0, size.1));
            self.redraw();
        }
        if pos != self.pos && !self.dragging {
            self.pos = pos;
            self.window.set_outer_position(PhysicalPosition::new(pos.0, pos.1));
        }
    }
}

fn detect_system_light(window: &Window) -> bool {
    if let Some(t) = window.theme() {
        return matches!(t, winit::window::Theme::Light);
    }
    #[cfg(target_os = "linux")]
    if let Some(light) = linux_portal_light() {
        return light;
    }
    false
}

#[cfg(target_os = "linux")]
fn linux_portal_light() -> Option<bool> {
    let conn = zbus::blocking::connection::Builder::session()
        .ok()?
        .method_timeout(Duration::from_millis(250))
        .build()
        .ok()?;
    let reply = conn
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Settings"),
            "Read",
            &("org.freedesktop.appearance", "color-scheme"),
        )
        .ok()?;
    let val: zbus::zvariant::OwnedValue = reply.body().deserialize().ok()?;
    let mut v: &zbus::zvariant::Value<'_> = &val;
    while let zbus::zvariant::Value::Value(inner) = v {
        v = inner;
    }
    match v {
        zbus::zvariant::Value::U32(1) => Some(false),
        zbus::zvariant::Value::U32(0 | 2) => Some(true),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Mon {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f64,
}

impl Mon {
    fn discover(el: &ActiveEventLoop) -> (Mon, Vec<Mon>) {
        let to_mon = |m: winit::monitor::MonitorHandle| {
            let (p, s) = (m.position(), m.size());
            Mon { x: p.x, y: p.y, w: s.width, h: s.height, scale: m.scale_factor() }
        };
        let list: Vec<Mon> = el.available_monitors().map(to_mon).collect();
        let primary = el
            .primary_monitor()
            .map(to_mon)
            .or_else(|| list.first().copied())
            .unwrap_or(Mon { x: 0, y: 0, w: 1920, h: 1080, scale: 1.0 });
        let list = if list.is_empty() { vec![primary] } else { list };
        (primary, list)
    }
}

fn monitor_at(pt: (i32, i32), primary: &Mon, mons: &[Mon]) -> Mon {
    mons.iter()
        .copied()
        .find(|m| pt.0 >= m.x && pt.0 < m.x + m.w as i32 && pt.1 >= m.y && pt.1 < m.y + m.h as i32)
        .or_else(|| {
            // Pick nearest monitor if point slightly spilled outside screen edges during WM drag.
            mons.iter().copied().min_by_key(|m| {
                let cx = m.x + m.w as i32 / 2;
                let cy = m.y + m.h as i32 / 2;
                (pt.0 as i64 - cx as i64).pow(2) + (pt.1 as i64 - cy as i64).pow(2)
            })
        })
        .unwrap_or(*primary)
}

/// Clamps or defaults the stationary compact-pill `(center_x, pill_top_y)` inside its monitor.
fn resolve_pill_anchor(pos: Option<(i32, i32)>, primary: &Mon, mons: &[Mon], font_pt: f32) -> (i32, i32) {
    let Some(raw) = pos else {
        return (primary.x + primary.w as i32 / 2, primary.y + (8.0 * primary.scale).round() as i32);
    };
    let mon = monitor_at(raw, primary, mons);
    let min_half = ((PILL_MIN_W as f64 * 0.5 + 6.0) * mon.scale).round() as i32;
    let ph = (island::pill_height(font_pt) as f64 * mon.scale).round() as i32;
    let cx = raw.0.clamp(mon.x + min_half, (mon.x + mon.w as i32 - min_half).max(mon.x + min_half));
    let top = raw.1.clamp(mon.y + 2, (mon.y + mon.h as i32 - ph - 2).max(mon.y + 2));
    (cx, top)
}

/// Computes the fixed `(canvas_size, window_pos, canvas_center_x, expand_up)` for the island window.
fn island_canvas_geometry(
    anchor: (i32, i32),
    cfg: &UiConfig,
    primary: &Mon,
    mons: &[Mon],
    scale: f64,
) -> ((u32, u32), (i32, i32), f32, bool) {
    let mon = monitor_at(anchor, primary, mons);
    let max_logical_w = cfg.island_max_width.max(EXPANDED_W + 32.0);
    let cw = ((max_logical_w as f64 * scale).ceil() as u32)
        .min((mon.w as f64 * 0.92) as u32)
        .max(200);
    let ch = (((EXPANDED_H + CANVAS_PAD_H) as f64 * scale).ceil() as u32)
        .min(mon.h.max(200));
    let ph = (island::pill_height(cfg.island_font_pt) as f64 * scale).round() as i32;

    let win_x = (anchor.0 - cw as i32 / 2)
        .clamp(mon.x, (mon.x + mon.w as i32 - cw as i32).max(mon.x));
    let expand_up = anchor.1 + ch as i32 > mon.y + mon.h as i32;
    let win_y = if expand_up {
        (anchor.1 + ph - ch as i32).max(mon.y)
    } else {
        anchor.1
    };
    let canvas_cx = (anchor.0 - win_x) as f32;
    ((cw, ch), (win_x, win_y), canvas_cx, expand_up)
}

fn desk_geometry(mon: &Mon, cfg: &UiConfig, scale: f64) -> ((u32, u32), (i32, i32)) {
    let w = (mon.w as f64 * 0.7) as u32;
    let h = (desktop::height(cfg.desktop_font_px) as f64 * scale).ceil() as u32;
    let pos = cfg.desktop_pos.unwrap_or((
        mon.x + (mon.w as i32 - w as i32) / 2,
        mon.y + (mon.h as f64 * 0.88) as i32 - h as i32,
    ));
    ((w, h), pos)
}

struct Art {
    src: Arc<model::Rgba>,
    gradient: Option<([u8; 3], [u8; 3])>,
    small: Option<Pixmap>,
    large: Option<Pixmap>,
}

impl Art {
    fn new(src: Arc<model::Rgba>) -> Self {
        let gradient = thumb(&src, 2).map(|q| {
            let px: [[u8; 3]; 4] = std::array::from_fn(|i| {
                let d = &q.data()[i * 4..];
                [d[0], d[1], d[2]]
            });
            let dist = |a: [u8; 3], b: [u8; 3]| (0..3).map(|i| (a[i] as i32 - b[i] as i32).pow(2)).sum::<i32>();
            let mut best = (px[0], px[1]);
            for a in px {
                for b in px {
                    if dist(a, b) > dist(best.0, best.1) {
                        best = (a, b);
                    }
                }
            }
            best
        });
        Self { src, gradient, small: None, large: None }
    }

    fn prepare(&mut self, small: u32, large: u32) {
        if self.small.as_ref().map(Pixmap::width) != Some(small) {
            self.small = thumb(&self.src, small);
        }
        if self.large.as_ref().map(Pixmap::width) != Some(large) {
            self.large = thumb(&self.src, large);
        }
    }
}

fn thumb(src: &model::Rgba, n: u32) -> Option<Pixmap> {
    let (w, h) = (src.width as usize, src.height as usize);
    if w == 0 || h == 0 || src.data.len() < w * h * 4 {
        return None;
    }
    let mut pm = Pixmap::new(n, n)?;
    let side = w.min(h);
    let (x0, y0) = ((w - side) / 2, (h - side) / 2);
    let n = n as usize;
    let span = |i: usize| (i * side / n, ((i + 1) * side / n).max(i * side / n + 1));
    for (i, px) in pm.data_mut().chunks_exact_mut(4).enumerate() {
        let ((sx0, sx1), (sy0, sy1)) = (span(i % n), span(i / n));
        let mut acc = [0u32; 4];
        for y in sy0..sy1 {
            let row = &src.data[((y0 + y) * w + x0 + sx0) * 4..((y0 + y) * w + x0 + sx1) * 4];
            for p in row.chunks_exact(4) {
                let a = p[3] as u32;
                acc[0] += p[0] as u32 * a;
                acc[1] += p[1] as u32 * a;
                acc[2] += p[2] as u32 * a;
                acc[3] += a;
            }
        }
        let cnt = ((sx1 - sx0) * (sy1 - sy0)) as u32;
        px[0] = (acc[0] / 255 / cnt) as u8;
        px[1] = (acc[1] / 255 / cnt) as u8;
        px[2] = (acc[2] / 255 / cnt) as u8;
        px[3] = (acc[3] / cnt) as u8;
    }
    Some(pm)
}

fn pick(lines: &[Line], t: i64) -> (Option<usize>, bool) {
    let Some(i) = lines.partition_point(|l| l.start_ms as i64 <= t).checked_sub(1) else {
        return (None, false);
    };
    let (cur, next) = (&lines[i], lines.get(i + 1));
    let start = cur.start_ms as i64;
    let end = match (cur.end_ms, next) {
        (Some(e), Some(n)) if n.start_ms as i64 - (e as i64) < 3000 => Some(n.start_ms as i64),
        (e, _) => e.map(|e| e as i64),
    };
    let max = match (end, next) {
        (Some(e), _) => e - start,
        (None, Some(n)) => (n.start_ms as i64 - start).min(9000),
        (None, None) => 8000,
    };
    (Some(i), !cur.text.is_empty() && t < start + max)
}

fn sung(line: &Line, t: i64) -> (Range<usize>, f32) {
    let mut off = 0;
    let mut res = (0..0, 0.0);
    for w in &line.words {
        if w.start_ms as i64 > t {
            break;
        }
        let span = w.end_ms.saturating_sub(w.start_ms).max(1) as f32;
        res = (off..off + w.text.trim_end().len(), ((t - w.start_ms as i64) as f32 / span).min(1.0));
        off += w.text.len();
    }
    res
}

fn fmt_time(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

#[derive(Clone, Copy)]
struct Anim {
    from: f32,
    to: f32,
    start: Instant,
    dur: f32,
    ease: fn(f32) -> f32,
}

impl Anim {
    fn new(v: f32, now: Instant) -> Self {
        Self { from: v, to: v, start: now, dur: 0.0, ease: ease_out_quad }
    }

    fn value(&self, now: Instant) -> f32 {
        if now < self.start {
            return self.from;
        }
        let e = (now - self.start).as_secs_f32();
        if e >= self.dur {
            return self.to;
        }
        self.from + (self.to - self.from) * (self.ease)(e / self.dur)
    }

    fn set(&mut self, to: f32, ms: u64, delay_ms: u64, ease: fn(f32) -> f32, now: Instant) {
        self.from = self.value(now);
        self.to = to;
        self.start = now + Duration::from_millis(delay_ms);
        self.dur = ms as f32 / 1000.0;
        self.ease = ease;
    }

    fn jump(&mut self, v: f32) {
        self.from = v;
        self.to = v;
        self.dur = 0.0;
    }

    fn done(&self, now: Instant) -> bool {
        now >= self.start + Duration::from_secs_f32(self.dur)
    }
}

fn ease_out_quad(p: f32) -> f32 {
    1.0 - (1.0 - p) * (1.0 - p)
}

fn ease_out_quart(p: f32) -> f32 {
    1.0 - (1.0 - p).powi(4)
}

fn ease_in_out_quad(p: f32) -> f32 {
    if p < 0.5 { 2.0 * p * p } else { 1.0 - (2.0 - 2.0 * p).powi(2) / 2.0 }
}

/// Critically-settled Dynamic Island curve (~2.5% gentle settle, zero rubbery bounce).
fn ease_out_spring(p: f32) -> f32 {
    const C1: f32 = 0.52;
    1.0 + (C1 + 1.0) * (p - 1.0).powi(3) + C1 * (p - 1.0).powi(2)
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.min(w / 2.0).min(h / 2.0);
    let k = r * 0.4477;
    let (x1, y1) = (x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x1 - r, y);
    pb.cubic_to(x1 - k, y, x1, y + k, x1, y + r);
    pb.line_to(x1, y1 - r);
    pb.cubic_to(x1, y1 - k, x1 - k, y1, x1 - r, y1);
    pb.line_to(x + r, y1);
    pb.cubic_to(x + k, y1, x, y1 - k, x, y1 - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + k, x + k, y, x + r, y);
    pb.close();
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Word;

    fn line(start: u64, end: Option<u64>, text: &str) -> Line {
        Line { start_ms: start, end_ms: end, text: text.into(), words: Vec::new() }
    }

    #[test]
    fn picks_lines_like_the_extension() {
        let ended = [line(1000, Some(3000), "a"), line(5000, Some(6000), "b"), line(20000, None, "c")];
        assert_eq!(pick(&ended, 500), (None, false));
        assert_eq!(pick(&ended, 2000), (Some(0), true));
        assert_eq!(pick(&ended, 4500), (Some(0), true));
        assert_eq!(pick(&ended, 7000), (Some(1), false));
        assert_eq!(pick(&ended, 27999), (Some(2), true));
        assert_eq!(pick(&ended, 28000), (Some(2), false));

        let open = [line(0, None, "x"), line(30000, None, "y"), line(31000, None, "")];
        assert_eq!(pick(&open, 8999), (Some(0), true));
        assert_eq!(pick(&open, 9000), (Some(0), false));
        assert_eq!(pick(&open, 30500), (Some(1), true));
        assert_eq!(pick(&open, 31500), (Some(2), false));
    }

    #[test]
    fn bottom_screen_anchor_expands_upward_without_moving_pill() {
        let mon = Mon { x: 0, y: 0, w: 1366, h: 768, scale: 1.0 };
        let cfg = UiConfig::default();
        let anchor = resolve_pill_anchor(Some((683, 661)), &mon, &[mon], cfg.island_font_pt);
        let (size, pos, cx, expand_up) = island_canvas_geometry(anchor, &cfg, &mon, &[mon], 1.0);
        assert!(expand_up, "pill at y=661 on 768px screen expands upward");
        let isl = Island::new(Instant::now());
        let (cox, coy, cw, ch) = isl.compact_rect(size, 1.0, cx, expand_up);
        // Screen Y of the compact pill matches anchor.1 (661) and screen center X matches anchor.0 (683).
        assert_eq!(pos.1 + coy as i32, anchor.1);
        assert_eq!(pos.0 + (cox + cw / 2.0).round() as i32, anchor.0);
        assert_eq!(ch as i32, island::pill_height(cfg.island_font_pt) as i32);
    }

    #[test]
    fn karaoke_progress() {
        let mut l = line(0, None, "Hello world");
        l.words = vec![
            Word { start_ms: 0, end_ms: 400, text: "Hello ".into() },
            Word { start_ms: 500, end_ms: 1500, text: "world".into() },
        ];
        assert_eq!(sung(&l, 200), (0..5, 0.5));
        assert_eq!(sung(&l, 450), (0..5, 1.0));
        assert_eq!(sung(&l, 1000), (6..11, 0.5));
        let mut late = l.clone();
        late.start_ms = 100;
        late.words[0].start_ms = 100;
        assert_eq!(sung(&late, 50), (0..0, 0.0));
    }

    #[test]
    fn thumb_crops_and_averages() {
        let mut data = Vec::new();
        for _ in 0..2 {
            for c in [[255, 0, 0], [255, 0, 0], [0, 0, 255], [0, 0, 255]] {
                data.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let t = thumb(&model::Rgba { width: 4, height: 2, data }, 1).unwrap();
        assert_eq!(t.data(), &[127, 0, 127, 255]);
    }
}
