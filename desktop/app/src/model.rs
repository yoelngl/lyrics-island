//! Types shared by media, lyrics, ui and the app shell. Changing them is a cross-module contract change.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

/// Album art as the platform hands it over; decoding happens in the app shell.
#[derive(Clone, Debug, PartialEq)]
pub enum ArtRef {
    /// http(s):// or file:// URL.
    Url(String),
    /// Encoded image bytes (PNG/JPEG) from GSMTC or MediaRemote.
    Bytes(Arc<Vec<u8>>),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// 0 when the player doesn't know yet.
    pub length_ms: u64,
    pub art: Option<ArtRef>,
    /// file:// URL of the playing media when local; used to find a sibling .lrc.
    pub file_url: Option<String>,
    /// Player identity (MPRIS bus name, GSMTC app id, macOS bundle id).
    pub player: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerChoice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    Stopped,
}

#[derive(Clone, Debug)]
pub struct PlayerState {
    pub track: Track,
    pub status: Status,
    /// Position sampled at `at`.
    pub position_ms: u64,
    pub at: Instant,
    /// Playback rate, 1.0 normally.
    pub rate: f64,
}

impl PlayerState {
    /// Extrapolated position right now.
    pub fn position_now(&self) -> u64 {
        if self.status != Status::Playing {
            return self.position_ms;
        }
        let elapsed = self.at.elapsed().as_secs_f64() * 1000.0 * self.rate;
        let pos = self.position_ms as f64 + elapsed.max(0.0);
        if self.track.length_ms > 0 {
            pos.min(self.track.length_ms as f64) as u64
        } else {
            pos as u64
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    /// Target playback position in milliseconds.
    Seek(u64),
}

#[derive(Clone, Debug)]
pub enum MediaEvent {
    /// Active player state, or None when no player is active.
    State(Option<PlayerState>),
    /// Active players currently discovered on the system.
    Players(Vec<PlayerChoice>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub start_ms: u64,
    pub end_ms: u64,
    /// Includes its trailing whitespace so concatenating words gives `Line::text`.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub start_ms: u64,
    /// When the singing of this line ends, if the source says so.
    pub end_ms: Option<u64>,
    pub text: String,
    /// Word timings when the source has them (TTML spans, enhanced LRC); empty otherwise.
    pub words: Vec<Word>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    /// Sorted by start_ms; only lines with text.
    pub lines: Vec<Line>,
    /// e.g. "lrc.red", "Better Lyrics", "Local file".
    pub source: String,
}

/// Decoded straight-alpha RGBA8 image.
#[derive(Clone, Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowKind {
    Island,
    Desktop,
    Settings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

/// What the UI needs from the user config. The app shell owns persistence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub island_visible: bool,
    pub desktop_visible: bool,
    /// Both overlays ignore the mouse.
    pub click_through: bool,
    /// Added to the player position before picking a line.
    pub offset_ms: i64,
    /// Physical top-left; None = default placement (island top-center, desktop bottom-center of primary monitor).
    pub island_pos: Option<(i32, i32)>,
    pub desktop_pos: Option<(i32, i32)>,
    /// Word-by-word highlight when word timings exist.
    pub karaoke: bool,
    pub desktop_font_px: f32,
    /// Widest the compact pill grows in logical px (200..=1400).
    pub island_max_width: f32,
    /// Compact lyric line font size in pt (8..=18).
    pub island_font_pt: f32,
    /// Color theme for the island and settings window.
    pub theme: Theme,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            island_visible: true,
            desktop_visible: false,
            click_through: false,
            offset_ms: 0,
            island_pos: None,
            desktop_pos: None,
            karaoke: true,
            desktop_font_px: 34.0,
            island_max_width: 850.0,
            island_font_pt: 11.0,
            theme: Theme::System,
        }
    }
}

impl UiConfig {
    pub fn sanitized(mut self) -> Self {
        self.offset_ms = self.offset_ms.clamp(-30_000, 30_000);
        self.desktop_font_px = if self.desktop_font_px.is_finite() {
            self.desktop_font_px.clamp(16.0, 96.0)
        } else {
            34.0
        };
        self.island_max_width = if self.island_max_width.is_finite() {
            self.island_max_width.clamp(200.0, 1400.0)
        } else {
            850.0
        };
        self.island_font_pt = if self.island_font_pt.is_finite() {
            self.island_font_pt.clamp(8.0, 18.0)
        } else {
            11.0
        };
        self
    }
}

/// What the UI asks the app shell to do in response to user input.
#[derive(Clone, Debug, PartialEq)]
pub enum UiAction {
    Player(Command),
    /// The user dragged an overlay window; persist the new physical top-left.
    Moved(WindowKind, (i32, i32)),
    SetUiConfig(UiConfig),
    ResetPositions,
    SetLrcDirs(Vec<PathBuf>),
    SetHotkeys(crate::config::Hotkeys),
    SetAutostart(bool),
    SelectPlayer(Option<String>),
}
