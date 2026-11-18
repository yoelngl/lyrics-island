use std::sync::Mutex;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::event_loop::EventLoopProxy;

use crate::model::{PlayerChoice, Theme, UiConfig};
use crate::{Action, AppEvent};

pub struct Tray {
    _icon: TrayIcon,
    island: CheckMenuItem,
    desktop: CheckMenuItem,
    click_through: CheckMenuItem,
    karaoke: CheckMenuItem,
    theme_system: CheckMenuItem,
    theme_dark: CheckMenuItem,
    theme_light: CheckMenuItem,
    player_menu: Submenu,
    player_auto: CheckMenuItem,
    player_items: Vec<(PlayerChoice, CheckMenuItem)>,
    offset: MenuItem,
    autostart: CheckMenuItem,
}

fn action(id: &str) -> Option<Action> {
    if let Some(rest) = id.strip_prefix("player:") {
        return Some(if rest == "auto" {
            Action::SelectPlayer(None)
        } else {
            Action::SelectPlayer(Some(rest.to_owned()))
        });
    }
    Some(match id {
        "settings" => Action::OpenSettings,
        "island" => Action::ToggleIsland,
        "desktop" => Action::ToggleDesktop,
        "click-through" => Action::ToggleClickThrough,
        "karaoke" => Action::ToggleKaraoke,
        "theme:system" => Action::SetTheme(Theme::System),
        "theme:dark" => Action::SetTheme(Theme::Dark),
        "theme:light" => Action::SetTheme(Theme::Light),
        "offset-earlier" => Action::OffsetBy(250),
        "offset-later" => Action::OffsetBy(-250),
        "offset-reset" => Action::OffsetReset,
        "reset-positions" => Action::ResetPositions,
        "autostart" => Action::ToggleAutostart,
        "open-config" => Action::OpenConfig,
        "quit" => Action::Quit,
        _ => return None,
    })
}

impl Tray {
    /// Must run on the main thread after the event loop started (macOS requirement).
    pub fn new(
        cfg: &UiConfig,
        autostart: bool,
        preferred_player: Option<&str>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> Option<Tray> {
        let settings = MenuItem::with_id("settings", "Settings...", true, None);
        let island = CheckMenuItem::with_id("island", "Show island", true, cfg.island_visible, None);
        let desktop = CheckMenuItem::with_id("desktop", "Desktop lyrics", true, cfg.desktop_visible, None);
        let click_through =
            CheckMenuItem::with_id("click-through", "Click-through", true, cfg.click_through, None);
        let karaoke = CheckMenuItem::with_id("karaoke", "Karaoke highlight", true, cfg.karaoke, None);

        let theme_system = CheckMenuItem::with_id(
            "theme:system",
            "Follow system",
            true,
            cfg.theme == Theme::System,
            None,
        );
        let theme_dark =
            CheckMenuItem::with_id("theme:dark", "Dark", true, cfg.theme == Theme::Dark, None);
        let theme_light =
            CheckMenuItem::with_id("theme:light", "Light", true, cfg.theme == Theme::Light, None);
        let theme_menu = Submenu::with_items(
            "Theme",
            true,
            &[&theme_system, &theme_dark, &theme_light],
        )
        .ok()?;

        let player_auto = CheckMenuItem::with_id(
            "player:auto",
            "Automatic",
            true,
            preferred_player.is_none(),
            None,
        );
        let player_menu = Submenu::with_items("Player", true, &[&player_auto]).ok()?;

        let offset = MenuItem::new(offset_label(cfg.offset_ms), false, None);
        let offset_menu = Submenu::with_items(
            "Lyrics offset",
            true,
            &[
                &offset,
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id("offset-earlier", "Show lines earlier (+250 ms)", true, None),
                &MenuItem::with_id("offset-later", "Show lines later (\u{2212}250 ms)", true, None),
                &MenuItem::with_id("offset-reset", "Reset", true, None),
            ],
        )
        .ok()?;

        let reset_positions =
            MenuItem::with_id("reset-positions", "Reset window positions", true, None);
        let autostart_item =
            CheckMenuItem::with_id("autostart", "Launch at login", true, autostart, None);

        let menu = Menu::with_items(&[
            &settings,
            &PredefinedMenuItem::separator(),
            &island,
            &desktop,
            &click_through,
            &karaoke,
            &theme_menu,
            &player_menu,
            &offset_menu,
            &reset_positions,
            &PredefinedMenuItem::separator(),
            &autostart_item,
            &MenuItem::with_id("open-config", "Open config file", true, None),
            &MenuItem::with_id("quit", "Quit", true, None),
        ])
        .ok()?;

        let icon = TrayIconBuilder::new()
            .with_icon(Icon::from_rgba(icon_rgba(), ICON_SIZE as u32, ICON_SIZE as u32).expect("valid icon"))
            .with_tooltip("Lyrics Island")
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| eprintln!("lyrics-island: tray icon unavailable: {e}"))
            .ok()?;

        // The proxy isn't Sync on every platform; the handler must be.
        let proxy = Mutex::new(proxy);
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some(a) = action(&e.id.0) {
                let _ = proxy.lock().unwrap().send_event(AppEvent::Action(a));
            }
        }));

        Some(Tray {
            _icon: icon,
            island,
            desktop,
            click_through,
            karaoke,
            theme_system,
            theme_dark,
            theme_light,
            player_menu,
            player_auto,
            player_items: Vec::new(),
            offset,
            autostart: autostart_item,
        })
    }

    /// Native menus may have toggled a check on click already; always mirror the config.
    pub fn sync(&self, cfg: &UiConfig, autostart: bool, preferred_player: Option<&str>) {
        self.island.set_checked(cfg.island_visible);
        self.desktop.set_checked(cfg.desktop_visible);
        self.click_through.set_checked(cfg.click_through);
        self.karaoke.set_checked(cfg.karaoke);
        self.theme_system.set_checked(cfg.theme == Theme::System);
        self.theme_dark.set_checked(cfg.theme == Theme::Dark);
        self.theme_light.set_checked(cfg.theme == Theme::Light);
        self.player_auto.set_checked(preferred_player.is_none());
        for (p, item) in &self.player_items {
            item.set_checked(preferred_player == Some(p.id.as_str()));
        }
        self.offset.set_text(offset_label(cfg.offset_ms));
        self.autostart.set_checked(autostart);
    }

    pub fn set_players(&mut self, players: &[PlayerChoice], preferred_player: Option<&str>) {
        if self.player_items.iter().map(|(p, _)| p).eq(players.iter()) {
            self.player_auto.set_checked(preferred_player.is_none());
            for (p, item) in &self.player_items {
                item.set_checked(preferred_player == Some(p.id.as_str()));
            }
            return;
        }

        // Keep "Automatic" at index 0 and rebuild dynamic player entries behind it.
        while self.player_menu.remove_at(1).is_some() {}
        self.player_items.clear();
        self.player_auto.set_checked(preferred_player.is_none());

        for p in players {
            let raw = if p.name.is_empty() { &p.id } else { &p.name };
            // Muda treats single '&' as a mnemonic prefix on Windows and GTK.
            let label = raw.replace('&', "&&");
            let checked = preferred_player == Some(p.id.as_str());
            let item = CheckMenuItem::with_id(format!("player:{}", p.id), label, true, checked, None);
            if self.player_menu.append(&item).is_ok() {
                self.player_items.push((p.clone(), item));
            }
        }
    }
}

fn offset_label(ms: i64) -> String {
    format!("Current: {ms:+} ms")
}

const ICON_SIZE: usize = 32;

/// White eighth note on a violet disc, antialiased by distance coverage.
fn icon_rgba() -> Vec<u8> {
    let cov = |d: f32| d.clamp(0.0, 1.0);
    let mut px = Vec::with_capacity(ICON_SIZE * ICON_SIZE * 4);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let disc = cov(16.0 - (fx - 16.0).hypot(fy - 16.0));
            let head = cov(4.0 - (fx - 13.5).hypot(fy - 21.5));
            let stem = ((15.5..17.5).contains(&fx) && (8.0..21.5).contains(&fy)) as u8 as f32;
            let flag = ((17.5..22.0).contains(&fx) && (8.0..11.0).contains(&fy)) as u8 as f32;
            let note = head.max(stem).max(flag);
            for bg in [0x7c, 0x4d, 0xff] {
                px.push((bg as f32 + (255.0 - bg as f32) * note).round() as u8);
            }
            px.push((disc * 255.0).round() as u8);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_map_to_actions() {
        assert_eq!(action("settings"), Some(Action::OpenSettings));
        assert_eq!(action("island"), Some(Action::ToggleIsland));
        assert_eq!(action("desktop"), Some(Action::ToggleDesktop));
        assert_eq!(action("click-through"), Some(Action::ToggleClickThrough));
        assert_eq!(action("karaoke"), Some(Action::ToggleKaraoke));
        assert_eq!(action("theme:system"), Some(Action::SetTheme(Theme::System)));
        assert_eq!(action("theme:dark"), Some(Action::SetTheme(Theme::Dark)));
        assert_eq!(action("theme:light"), Some(Action::SetTheme(Theme::Light)));
        assert_eq!(action("player:auto"), Some(Action::SelectPlayer(None)));
        assert_eq!(
            action("player:org.mpris.MediaPlayer2.spotify"),
            Some(Action::SelectPlayer(Some("org.mpris.MediaPlayer2.spotify".into())))
        );
        assert_eq!(action("offset-earlier"), Some(Action::OffsetBy(250)));
        assert_eq!(action("offset-later"), Some(Action::OffsetBy(-250)));
        assert_eq!(action("offset-reset"), Some(Action::OffsetReset));
        assert_eq!(action("reset-positions"), Some(Action::ResetPositions));
        assert_eq!(action("autostart"), Some(Action::ToggleAutostart));
        assert_eq!(action("open-config"), Some(Action::OpenConfig));
        assert_eq!(action("quit"), Some(Action::Quit));
        assert_eq!(action("unknown"), None);
    }
}
