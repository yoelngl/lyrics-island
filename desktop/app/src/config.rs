use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::model::UiConfig;
use crate::Action;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ui: UiConfig,
    /// Extra folders searched for `Artist - Title.lrc` files.
    pub lrc_dirs: Vec<PathBuf>,
    pub hotkeys: Hotkeys,
    /// Pinned player id (bus name / app id), or None for automatic choice.
    pub preferred_player: Option<String>,
}

/// Accelerator strings in global-hotkey syntax, e.g. "Ctrl+Alt+L"; empty disables.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub toggle_island: String,
    pub toggle_desktop: String,
    pub toggle_click_through: String,
    pub play_pause: String,
    pub offset_earlier: String,
    pub offset_later: String,
    pub offset_reset: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            toggle_island: "Ctrl+Alt+L".into(),
            toggle_desktop: "Ctrl+Alt+D".into(),
            toggle_click_through: "Ctrl+Alt+T".into(),
            play_pause: "Ctrl+Alt+P".into(),
            offset_earlier: "Ctrl+Alt+Left".into(),
            offset_later: "Ctrl+Alt+Right".into(),
            offset_reset: "Ctrl+Alt+0".into(),
        }
    }
}

impl Hotkeys {
    pub fn bindings(&self) -> [(&str, Action); 7] {
        [
            (self.toggle_island.as_str(), Action::ToggleIsland),
            (self.toggle_desktop.as_str(), Action::ToggleDesktop),
            (self.toggle_click_through.as_str(), Action::ToggleClickThrough),
            (self.play_pause.as_str(), Action::PlayPause),
            // offset_ms is added to the position, so a positive offset shows lines earlier.
            (self.offset_earlier.as_str(), Action::OffsetBy(250)),
            (self.offset_later.as_str(), Action::OffsetBy(-250)),
            (self.offset_reset.as_str(), Action::OffsetReset),
        ]
    }

    pub fn field_mut(&mut self, index: usize) -> Option<&mut String> {
        Some(match index {
            0 => &mut self.toggle_island,
            1 => &mut self.toggle_desktop,
            2 => &mut self.toggle_click_through,
            3 => &mut self.play_pause,
            4 => &mut self.offset_earlier,
            5 => &mut self.offset_later,
            6 => &mut self.offset_reset,
            _ => return None,
        })
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("lyrics-island")
            .join("config.json")
    }

    pub fn sanitized(mut self) -> Self {
        self.ui = self.ui.sanitized();
        if self.preferred_player.as_deref().is_some_and(|s| s.trim().is_empty()) {
            self.preferred_player = None;
        }
        self
    }

    pub fn load() -> (Config, Option<SystemTime>) {
        let path = Self::path();
        let cfg = Self::load_from(&path);
        let mtime = file_mtime(&path);
        (cfg, mtime)
    }

    pub fn save(&self) -> Option<SystemTime> {
        let path = Self::path();
        self.save_to(&path);
        file_mtime(&path)
    }

    /// Reads an externally edited config when its mtime changed; leaves partial/invalid edits untouched.
    pub fn reload_if_changed(last_mtime: &mut Option<SystemTime>) -> Option<Config> {
        Self::reload_from(&Self::path(), last_mtime)
    }

    fn reload_from(path: &Path, last_mtime: &mut Option<SystemTime>) -> Option<Config> {
        let mtime = file_mtime(path)?;
        if *last_mtime == Some(mtime) {
            return None;
        }
        let bytes = fs::read(path).ok()?;
        let cfg = serde_json::from_slice::<Config>(&bytes).ok()?.sanitized();
        *last_mtime = Some(mtime);
        Some(cfg)
    }

    /// Missing or corrupt files yield defaults, which are written back so the user has something to edit.
    fn load_from(path: &Path) -> Config {
        let cfg = match fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Config>(&bytes) {
                Ok(cfg) => cfg.sanitized(),
                Err(e) => {
                    let bak = path.with_extension("json.bak");
                    eprintln!("lyrics-island: {} is invalid ({e}); moved to {}", path.display(), bak.display());
                    let _ = fs::rename(path, &bak);
                    Config::default()
                }
            },
            Err(e) if e.kind() == ErrorKind::NotFound => Config::default(),
            Err(e) => {
                eprintln!("lyrics-island: can't read {}: {e}", path.display());
                return Config::default();
            }
        };
        cfg.save_to(path);
        cfg
    }

    fn save_to(&self, path: &Path) {
        let tmp = path.with_extension("json.tmp");
        let res = path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|_| fs::write(&tmp, serde_json::to_vec_pretty(self).expect("config serializes")))
            .and_then(|_| fs::rename(&tmp, path));
        if let Err(e) = res {
            eprintln!("lyrics-island: can't save {}: {e}", path.display());
        }
    }
}

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Theme;

    #[test]
    fn round_trip_recovery_and_live_reload() {
        let dir = std::env::temp_dir().join(format!("lyrics-island-cfg-{}", std::process::id()));
        let path = dir.join("config.json");
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(Config::load_from(&path), Config::default());
        assert!(path.exists(), "defaults written back");

        let mut cfg = Config::default();
        cfg.ui.offset_ms = -750;
        cfg.ui.island_pos = Some((10, -20));
        cfg.ui.theme = Theme::Light;
        cfg.ui.island_max_width = 1100.0;
        cfg.ui.island_font_pt = 14.0;
        cfg.preferred_player = Some("org.mpris.MediaPlayer2.spotify".into());
        cfg.lrc_dirs.push(PathBuf::from("/music/lrc"));
        cfg.hotkeys.play_pause = "Ctrl+Shift+Space".into();
        cfg.save_to(&path);
        assert_eq!(Config::load_from(&path), cfg);

        let mut mtime = None;
        let reloaded = Config::reload_from(&path, &mut mtime).unwrap();
        assert_eq!(reloaded, cfg);
        assert!(Config::reload_from(&path, &mut mtime).is_none(), "unchanged mtime skips reload");

        fs::write(&path, r#"{"ui":{"karaoke":false,"island_font_pt":99,"theme":"dark"}}"#).unwrap();
        let partial = Config::load_from(&path);
        assert!(!partial.ui.karaoke);
        assert_eq!(partial.ui.island_font_pt, 18.0);
        assert_eq!(partial.ui.theme, Theme::Dark);
        assert_eq!(partial.hotkeys, Hotkeys::default());

        fs::write(&path, "{not json").unwrap();
        assert_eq!(Config::load_from(&path), Config::default());
        assert!(path.with_extension("json.bak").exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn default_hotkeys_parse() {
        for (s, _) in Hotkeys::default().bindings() {
            s.parse::<global_hotkey::hotkey::HotKey>().unwrap();
        }
    }
}
