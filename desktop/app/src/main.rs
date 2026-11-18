#![cfg_attr(windows, windows_subsystem = "windows")]

mod art;
mod autostart;
mod config;
mod hotkeys;
mod instance;
mod lyrics;
mod media;
mod model;
mod tray;
mod ui;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::WindowId;

use config::Config;
use model::{ArtRef, Command, Lyrics, MediaEvent, PlayerState, Rgba, Theme, Track, UiAction, WindowKind};

/// User intents from the tray menu and global hotkeys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    OpenSettings,
    ToggleIsland,
    ToggleLock,
    ToggleDesktop,
    ToggleClickThrough,
    ToggleKaraoke,
    SetTheme(Theme),
    SelectPlayer(Option<String>),
    PlayPause,
    OffsetBy(i64),
    OffsetReset,
    ResetPositions,
    ToggleAutostart,
    OpenConfig,
    Quit,
}

pub enum AppEvent {
    Media(MediaEvent),
    /// Fetch result tagged with the generation it was started for.
    Lyrics(u64, Option<Lyrics>),
    Art(u64, Option<Arc<Rgba>>),
    Action(Action),
    ConfigFileChanged,
}

const SAVE_DEBOUNCE: Duration = Duration::from_secs(1);

struct App {
    proxy: EventLoopProxy<AppEvent>,
    cfg: Config,
    cfg_mtime: Option<SystemTime>,
    autostart: bool,
    open_settings_on_start: bool,
    ui: Option<ui::Ui>,
    media: Option<media::Media>,
    tray: Option<tray::Tray>,
    hotkeys: Option<hotkeys::Hotkeys>,
    track: Option<Track>,
    lyrics_gen: u64,
    cancel: Arc<AtomicBool>,
    art: Option<ArtRef>,
    art_gen: u64,
    /// Decoded art by URL; cleared on every new song.
    art_cache: HashMap<String, Arc<Rgba>>,
    save_at: Option<Instant>,
}

fn same_song(a: &Track, b: &Track) -> bool {
    a.title == b.title
        && a.artist == b.artist
        && a.album == b.album
        && a.length_ms == b.length_ms
        && a.player == b.player
}

impl App {
    fn on_state(&mut self, state: Option<PlayerState>) {
        if self.ui.is_none() {
            return;
        }
        let track = state.as_ref().map(|s| &s.track);
        let new_song = match (track, &self.track) {
            (Some(a), Some(b)) => !same_song(a, b),
            (a, b) => a.is_some() != b.is_some(),
        };

        if new_song {
            self.art_cache.clear();
            self.refetch_lyrics_for(track);
        }

        let art = track.and_then(|t| t.art.clone());
        if art != self.art {
            self.art_gen += 1;
            let cached = match &art {
                Some(ArtRef::Url(u)) => self.art_cache.get(u).cloned(),
                _ => None,
            };
            // Keep the old cover while a same-song replacement loads; never show the previous song's.
            if let Some(ui) = self.ui.as_mut() {
                if cached.is_some() || art.is_none() || new_song {
                    ui.set_art(cached.clone());
                }
            }
            if let (None, Some(a)) = (cached, art.clone()) {
                let (gen, proxy) = (self.art_gen, self.proxy.clone());
                std::thread::spawn(move || {
                    let img = art::load(&a).map(Arc::new);
                    let _ = proxy.send_event(AppEvent::Art(gen, img));
                });
            }
            self.art = art;
        }

        self.track = track.cloned();
        if let Some(ui) = self.ui.as_mut() {
            ui.set_player(state);
        }
    }

    fn refetch_lyrics_for(&mut self, track: Option<&Track>) {
        self.cancel.store(true, Ordering::Relaxed);
        self.lyrics_gen += 1;
        if let Some(ui) = self.ui.as_mut() {
            ui.set_lyrics(None);
        }
        if let Some(t) = track.filter(|t| !t.title.is_empty()) {
            let cancel = Arc::new(AtomicBool::new(false));
            self.cancel = cancel.clone();
            let (t, dirs, gen, proxy) = (t.clone(), self.cfg.lrc_dirs.clone(), self.lyrics_gen, self.proxy.clone());
            std::thread::spawn(move || {
                let lyrics = lyrics::fetch(&t, &dirs, &cancel);
                let _ = proxy.send_event(AppEvent::Lyrics(gen, lyrics));
            });
        }
    }

    fn on_action(&mut self, event_loop: &ActiveEventLoop, action: Action) {
        // Ignore global shortcuts while the user is recording a new shortcut inside Settings.
        if self.ui.as_ref().is_some_and(|u| u.is_recording_hotkey())
            && !matches!(action, Action::OpenSettings | Action::Quit)
        {
            return;
        }
        match action {
            Action::OpenSettings => {
                if let Some(ui) = self.ui.as_mut() {
                    ui.open_settings(event_loop);
                }
                return;
            }
            Action::ToggleIsland => self.cfg.ui.island_visible = !self.cfg.ui.island_visible,
            Action::ToggleLock => self.cfg.ui.island_locked = !self.cfg.ui.island_locked,
            Action::ToggleDesktop => self.cfg.ui.desktop_visible = !self.cfg.ui.desktop_visible,
            Action::ToggleClickThrough => self.cfg.ui.click_through = !self.cfg.ui.click_through,
            Action::ToggleKaraoke => self.cfg.ui.karaoke = !self.cfg.ui.karaoke,
            Action::SetTheme(theme) => self.cfg.ui.theme = theme,
            Action::SelectPlayer(id) => {
                self.set_preferred_player(id);
                return;
            }
            Action::OffsetBy(d) => self.cfg.ui.offset_ms = (self.cfg.ui.offset_ms + d).clamp(-30_000, 30_000),
            Action::OffsetReset => self.cfg.ui.offset_ms = 0,
            Action::ResetPositions => {
                self.cfg.ui.island_pos = None;
                self.cfg.ui.desktop_pos = None;
            }
            Action::ToggleAutostart => {
                self.set_autostart(!self.autostart);
                return;
            }
            Action::PlayPause => {
                if let Some(m) = &self.media {
                    m.send(Command::PlayPause);
                }
                return;
            }
            Action::OpenConfig => return open_config(),
            Action::Quit => return event_loop.exit(),
        }
        self.sync_all_and_save();
    }

    fn on_ui_action(&mut self, action: UiAction) {
        match action {
            UiAction::Player(cmd) => {
                if let Some(m) = &self.media {
                    m.send(cmd);
                }
            }
            UiAction::Moved(kind, pos) => {
                match kind {
                    WindowKind::Island => self.cfg.ui.island_pos = Some(pos),
                    WindowKind::Desktop => self.cfg.ui.desktop_pos = Some(pos),
                    WindowKind::Settings => return,
                }
                if let Some(ui) = self.ui.as_mut() {
                    ui.sync_settings(&self.cfg, self.autostart);
                }
                self.schedule_save();
            }
            UiAction::SetUiConfig(ui_cfg) => {
                self.cfg.ui = ui_cfg.sanitized();
                self.sync_all_and_save();
            }
            UiAction::ResetPositions => {
                self.cfg.ui.island_pos = None;
                self.cfg.ui.desktop_pos = None;
                self.sync_all_and_save();
            }
            UiAction::SetLrcDirs(dirs) => {
                if self.cfg.lrc_dirs != dirs {
                    self.cfg.lrc_dirs = dirs;
                    let cur = self.track.clone();
                    self.refetch_lyrics_for(cur.as_ref());
                    self.sync_all_and_save();
                }
            }
            UiAction::SetHotkeys(keys) => {
                self.cfg.hotkeys = keys;
                let errs = self.hotkeys.as_mut().map_or_else(Vec::new, |h| h.rebind(&self.cfg.hotkeys));
                if let Some(ui) = self.ui.as_mut() {
                    ui.set_hotkey_errors(errs);
                }
                self.sync_all_and_save();
            }
            UiAction::SetAutostart(enable) => self.set_autostart(enable),
            UiAction::SelectPlayer(id) => self.set_preferred_player(id),
        }
    }

    fn set_preferred_player(&mut self, id: Option<String>) {
        self.cfg.preferred_player = id.clone();
        if let Some(m) = &self.media {
            m.select_player(id);
        }
        self.sync_all_and_save();
    }

    fn set_autostart(&mut self, enable: bool) {
        let err = match autostart::set_enabled(enable) {
            Ok(()) => {
                self.autostart = autostart::is_enabled();
                None
            }
            Err(e) => {
                eprintln!("lyrics-island: autostart failed: {e}");
                self.autostart = autostart::is_enabled();
                Some(e)
            }
        };
        if let Some(ui) = self.ui.as_mut() {
            ui.set_autostart_error(err);
            ui.sync_settings(&self.cfg, self.autostart);
        }
        if let Some(t) = &self.tray {
            t.sync(&self.cfg.ui, self.autostart, self.cfg.preferred_player.as_deref());
        }
    }

    fn on_config_file_changed(&mut self) {
        if self.save_at.is_some() {
            return;
        }
        let Some(new_cfg) = Config::reload_if_changed(&mut self.cfg_mtime) else { return };
        if new_cfg == self.cfg {
            return;
        }
        let hotkeys_changed = new_cfg.hotkeys != self.cfg.hotkeys;
        let dirs_changed = new_cfg.lrc_dirs != self.cfg.lrc_dirs;
        let player_changed = new_cfg.preferred_player != self.cfg.preferred_player;
        self.cfg = new_cfg;

        if hotkeys_changed {
            let errs = self.hotkeys.as_mut().map_or_else(Vec::new, |h| h.rebind(&self.cfg.hotkeys));
            if let Some(ui) = self.ui.as_mut() {
                ui.set_hotkey_errors(errs);
            }
        }
        if player_changed {
            if let Some(m) = &self.media {
                m.select_player(self.cfg.preferred_player.clone());
            }
        }
        if dirs_changed {
            let cur = self.track.clone();
            self.refetch_lyrics_for(cur.as_ref());
        }
        if let Some(ui) = self.ui.as_mut() {
            ui.sync_settings(&self.cfg, self.autostart);
        }
        if let Some(t) = &self.tray {
            t.sync(&self.cfg.ui, self.autostart, self.cfg.preferred_player.as_deref());
        }
    }

    fn sync_all_and_save(&mut self) {
        if let Some(ui) = self.ui.as_mut() {
            ui.sync_settings(&self.cfg, self.autostart);
        }
        if let Some(t) = &self.tray {
            t.sync(&self.cfg.ui, self.autostart, self.cfg.preferred_player.as_deref());
        }
        self.schedule_save();
    }

    fn schedule_save(&mut self) {
        self.save_at.get_or_insert_with(|| Instant::now() + SAVE_DEBOUNCE);
    }
}

fn spawn_config_watcher(proxy: EventLoopProxy<AppEvent>, initial_mtime: Option<SystemTime>) {
    let path = Config::path();
    let _ = std::thread::Builder::new().name("config-watch".into()).spawn(move || {
        let mut seen = initial_mtime;
        loop {
            std::thread::sleep(Duration::from_millis(1500));
            let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            if mtime.is_some() && mtime != seen {
                seen = mtime;
                if proxy.send_event(AppEvent::ConfigFileChanged).is_err() {
                    return;
                }
            }
        }
    });
}

fn open_config() {
    let path = Config::path();
    #[cfg(target_os = "linux")]
    let res = std::process::Command::new("xdg-open").arg(&path).spawn();
    #[cfg(target_os = "macos")]
    let res = std::process::Command::new("open").arg("-t").arg(&path).spawn();
    #[cfg(windows)]
    let res = std::process::Command::new("notepad.exe").arg(&path).spawn();
    if let Err(e) = res {
        eprintln!("lyrics-island: can't open {}: {e}", path.display());
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.ui.is_some() {
            return;
        }
        let mut ui = ui::Ui::new(event_loop, &self.cfg, self.autostart);
        let (hk, hk_errs) = hotkeys::Hotkeys::new(&self.cfg.hotkeys, self.proxy.clone());
        ui.set_hotkey_errors(hk_errs);
        if self.open_settings_on_start {
            ui.open_settings(event_loop);
        }
        self.ui = Some(ui);
        self.hotkeys = Some(hk);
        self.tray = tray::Tray::new(
            &self.cfg.ui,
            self.autostart,
            self.cfg.preferred_player.as_deref(),
            self.proxy.clone(),
        );
        spawn_config_watcher(self.proxy.clone(), self.cfg_mtime);
        let proxy = self.proxy.clone();
        self.media = Some(media::Media::spawn(
            self.cfg.preferred_player.clone(),
            Box::new(move |e| {
                let _ = proxy.send_event(AppEvent::Media(e));
            }),
        ));
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Media(MediaEvent::State(state)) => self.on_state(state),
            AppEvent::Media(MediaEvent::Players(players)) => {
                if let Some(t) = self.tray.as_mut() {
                    t.set_players(&players, self.cfg.preferred_player.as_deref());
                }
                if let Some(ui) = self.ui.as_mut() {
                    ui.set_players(players);
                }
            }
            AppEvent::Lyrics(gen, lyrics) if gen == self.lyrics_gen => {
                if let Some(ui) = self.ui.as_mut() {
                    ui.set_lyrics(lyrics.map(Arc::new));
                }
            }
            AppEvent::Art(gen, img) if gen == self.art_gen => {
                if let (Some(ArtRef::Url(u)), Some(img)) = (&self.art, &img) {
                    self.art_cache.insert(u.clone(), img.clone());
                }
                if let Some(ui) = self.ui.as_mut() {
                    ui.set_art(img);
                }
            }
            AppEvent::Lyrics(..) | AppEvent::Art(..) => {}
            AppEvent::Action(a) => self.on_action(event_loop, a),
            AppEvent::ConfigFileChanged => self.on_config_file_changed(),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(ui) = self.ui.as_mut() else { return };
        let actions = ui.window_event(event_loop, id, event);
        for action in actions {
            self.on_ui_action(action);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.save_at.is_some_and(|t| t <= Instant::now()) {
            self.save_at = None;
            self.cfg_mtime = self.cfg.save();
        }
        let next = self.ui.as_mut().and_then(|ui| ui.update());
        let wake = match (next, self.save_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.cancel.store(true, Ordering::Relaxed);
        if self.save_at.take().is_some() {
            self.cfg_mtime = self.cfg.save();
        }
    }
}

fn main() {
    let open_settings_on_start = std::env::args().skip(1).any(|a| a == "--settings");
    let Some(mut instance_guard) = instance::InstanceGuard::try_lock(open_settings_on_start) else {
        return;
    };

    #[allow(unused_mut)]
    let mut builder = EventLoop::<AppEvent>::with_user_event();
    #[cfg(target_os = "linux")]
    {
        let force_wayland = std::env::var("LYRICS_ISLAND_BACKEND")
            .is_ok_and(|v| v.eq_ignore_ascii_case("wayland"));
        let has_x11 = std::env::var_os("DISPLAY").is_some();
        if force_wayland || !has_x11 {
            use winit::platform::wayland::EventLoopBuilderExtWayland;
            builder.with_wayland();
        } else {
            // X11 / XWayland lets overlays place themselves and stay above windows on every WM (incl. GNOME).
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }
    }
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    let event_loop = match builder.build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("lyrics-island: can't start the event loop: {e}");
            std::process::exit(1);
        }
    };
    let proxy = event_loop.create_proxy();
    instance_guard.spawn_ipc(proxy.clone());
    let (cfg, cfg_mtime) = Config::load();
    let mut app = App {
        proxy,
        cfg,
        cfg_mtime,
        autostart: autostart::is_enabled(),
        open_settings_on_start,
        ui: None,
        media: None,
        tray: None,
        hotkeys: None,
        track: None,
        lyrics_gen: 0,
        cancel: Arc::new(AtomicBool::new(false)),
        art: None,
        art_gen: 0,
        art_cache: HashMap::new(),
        save_at: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("lyrics-island: {e}");
        drop(instance_guard);
        std::process::exit(1);
    }
    drop(instance_guard);
}
