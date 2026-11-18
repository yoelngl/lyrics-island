//! Now-playing source per OS: MPRIS (Linux), GSMTC (Windows), MediaRemote via perl helper or AppleScript (macOS).
//! Each backend runs one thread that feeds `Emitter`, which only forwards real changes.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use crate::model::{Command, MediaEvent, PlayerChoice, PlayerState};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as platform;

#[cfg(not(target_os = "linux"))]
const POLL: Duration = Duration::from_millis(500);
/// Position disagreement with our own extrapolation that counts as a seek.
const SEEK_MS: f64 = 1500.0;

pub(crate) enum Msg {
    Cmd(Command),
    Select(Option<String>),
    #[cfg(target_os = "linux")]
    BusWake,
    #[cfg(target_os = "linux")]
    Seeked { sender: String, pos_ms: u64 },
}

pub struct Media {
    tx: Sender<Msg>,
}

impl Media {
    pub fn spawn(preferred: Option<String>, on_event: Box<dyn Fn(MediaEvent) + Send + 'static>) -> Media {
        let (tx, rx) = mpsc::channel();
        let tx_worker = tx.clone();
        std::thread::Builder::new()
            .name("media".into())
            .spawn(move || {
                let emit = Emitter { on_event, last: None, sent: false, players: Vec::new() };
                platform::run(tx_worker, rx, preferred, emit);
            })
            .expect("spawn media thread");
        Media { tx }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Cmd(cmd));
    }

    pub fn select_player(&self, id: Option<String>) {
        let _ = self.tx.send(Msg::Select(id));
    }
}

pub(crate) struct Emitter {
    on_event: Box<dyn Fn(MediaEvent) + Send + 'static>,
    last: Option<PlayerState>,
    sent: bool,
    players: Vec<PlayerChoice>,
}

impl Emitter {
    /// Last state forwarded to the app, which MPRIS needs to tell stale Position values from seeks.
    pub(crate) fn last(&self) -> Option<&PlayerState> {
        self.last.as_ref()
    }

    pub(crate) fn push(&mut self, state: Option<PlayerState>) {
        let changed = match (&self.last, &state) {
            (None, None) => !self.sent,
            (Some(a), Some(b)) => {
                a.track != b.track
                    || a.status != b.status
                    || a.rate != b.rate
                    || (a.position_now() as f64 - b.position_ms as f64).abs() > SEEK_MS
            }
            _ => true,
        };
        if changed {
            self.sent = true;
            self.last = state.clone();
            (self.on_event)(MediaEvent::State(state));
        }
    }

    /// Emits an explicit seek update immediately even if the jump is smaller than `SEEK_MS`.
    pub(crate) fn push_seek(&mut self, pos_ms: u64) {
        let Some(mut cur) = self.last.clone() else { return };
        cur.position_ms = if cur.track.length_ms > 0 { pos_ms.min(cur.track.length_ms) } else { pos_ms };
        cur.at = Instant::now();
        self.sent = true;
        self.last = Some(cur.clone());
        (self.on_event)(MediaEvent::State(Some(cur)));
    }

    pub(crate) fn push_players(&mut self, list: Vec<PlayerChoice>) {
        if self.players != list {
            self.players = list.clone();
            (self.on_event)(MediaEvent::Players(list));
        }
    }
}

/// Sleeps one poll interval, waking early for messages. Returns None once the app dropped `Media`.
pub(crate) fn wait(rx: &Receiver<Msg>, timeout: Duration) -> Option<Option<Msg>> {
    match rx.recv_timeout(timeout) {
        Ok(msg) => Some(Some(msg)),
        Err(mpsc::RecvTimeoutError::Timeout) => Some(None),
        Err(mpsc::RecvTimeoutError::Disconnected) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Status, Track};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[test]
    fn emits_only_on_change_seek_or_player_list_diff() {
        let count = Arc::new(Mutex::new(0));
        let c = count.clone();
        let mut e = Emitter {
            on_event: Box::new(move |_| *c.lock().unwrap() += 1),
            last: None,
            sent: false,
            players: Vec::new(),
        };
        let state = |pos, status| PlayerState {
            track: Track { title: "t".into(), length_ms: 200_000, ..Default::default() },
            status,
            position_ms: pos,
            at: Instant::now(),
            rate: 1.0,
        };
        e.push(None);
        e.push(None);
        assert_eq!(*count.lock().unwrap(), 1);
        e.push(Some(state(1000, Status::Paused)));
        e.push(Some(state(1200, Status::Paused)));
        assert_eq!(*count.lock().unwrap(), 2);
        // Explicit seek under 1500 ms still emits immediately.
        e.push_seek(1500);
        assert_eq!(*count.lock().unwrap(), 3);
        e.push(Some(state(5000, Status::Paused)));
        assert_eq!(*count.lock().unwrap(), 4);
        e.push(Some(state(5000, Status::Playing)));
        assert_eq!(*count.lock().unwrap(), 5);

        let p = vec![PlayerChoice { id: "a".into(), name: "Spotify".into() }];
        e.push_players(p.clone());
        e.push_players(p);
        assert_eq!(*count.lock().unwrap(), 6);
    }
}
