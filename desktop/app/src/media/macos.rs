//! MediaRemote through the mr-helper dylib hosted by /usr/bin/perl (entitled on macOS 15.4+), falling back to
//! scripting Spotify/Music through osascript when the helper is missing, silent or a specific app is pinned.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command as Process, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Deserialize;

use super::{wait, Emitter, Msg, POLL};
use crate::model::{ArtRef, Command, PlayerChoice, PlayerState, Status, Track};

const PERL_LOADER: &str = "use DynaLoader; my $h = DynaLoader::dl_load_file($ARGV[0], 0) or die; \
    my $s = DynaLoader::dl_find_symbol($h, $ARGV[1]) or die; DynaLoader::dl_install_xsub('main::run', $s); run();";
const HELPER_SILENCE: Duration = Duration::from_secs(3);
const SCRIPT_POLL: Duration = Duration::from_secs(1);

/// JXA instead of AppleScript so a missing Spotify/Music fails at runtime inside `try` rather than breaking compilation
/// of the whole script; `running()` keeps it from ever launching them.
const POLL_SCRIPT: &str = r#"(() => {
  const preferred = $.NSProcessInfo.processInfo.environment.objectForKey("LYRICS_ISLAND_PLAYER").js || "";
  let chosen = null;
  const active = [];
  for (const name of ["Spotify", "Music"]) {
    try {
      const app = Application(name);
      if (!app.running()) continue;
      const state = app.playerState();
      if (state !== "playing" && state !== "paused") continue;
      const t = app.currentTrack();
      active.push(name);
      const item = {
        player: name,
        title: t.name(),
        artist: t.artist(),
        album: t.album(),
        duration_ms: name === "Spotify" ? t.duration() : t.duration() * 1000,
        elapsed_ms: app.playerPosition() * 1000,
        rate: state === "playing" ? 1 : 0,
        art_url: name === "Spotify" ? t.artworkUrl() : "",
      };
      if (preferred === name) chosen = item;
      else if (!preferred && state === "playing" && (!chosen || chosen.rate === 0)) chosen = item;
      else if (!preferred && !chosen) chosen = item;
    } catch (e) {}
  }
  return JSON.stringify({ info: chosen, players: active });
})()"#;

#[derive(Deserialize, Default)]
#[serde(default)]
struct Info {
    player: String,
    title: String,
    artist: String,
    album: String,
    duration_ms: f64,
    elapsed_ms: f64,
    rate: f64,
    /// Helper only: new artwork, "" when cleared, absent when unchanged.
    artwork_b64: Option<String>,
    /// Script only.
    art_url: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ScriptPoll {
    info: Option<Info>,
    players: Vec<String>,
}

impl Info {
    fn into_state(self, art: Option<ArtRef>) -> Option<PlayerState> {
        if self.title.is_empty() {
            return None;
        }
        Some(PlayerState {
            track: Track {
                title: self.title,
                artist: self.artist,
                album: self.album,
                length_ms: self.duration_ms.max(0.0) as u64,
                art,
                file_url: None,
                player: if self.player.is_empty() { "MediaRemote".into() } else { self.player },
            },
            status: if self.rate > 0.0 { Status::Playing } else { Status::Paused },
            position_ms: self.elapsed_ms.max(0.0) as u64,
            at: Instant::now(),
            rate: if self.rate > 0.0 { self.rate } else { 1.0 },
        })
    }
}

struct Helper {
    dylib: PathBuf,
    child: Child,
    states: Receiver<Option<PlayerState>>,
    started: Instant,
    heard: bool,
}

pub(super) fn run(_tx: Sender<Msg>, rx: Receiver<Msg>, mut preferred: Option<String>, mut emit: Emitter) {
    let mut helper = find_dylib().and_then(spawn_helper);
    let mut scripted: Option<String> = None;
    loop {
        let mut helper_failed = false;
        let force_script = matches!(preferred.as_deref(), Some("Spotify" | "Music"));
        if let Some(h) = &mut helper {
            loop {
                match h.states.try_recv() {
                    Ok(state) => {
                        h.heard = true;
                        if !force_script {
                            let choices = state
                                .as_ref()
                                .map(|_| vec![PlayerChoice { id: "MediaRemote".into(), name: "System Now Playing".into() }])
                                .unwrap_or_default();
                            emit.push_players(choices);
                            emit.push(state);
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        helper_failed = true;
                        break;
                    }
                }
            }
            helper_failed |= !h.heard && h.started.elapsed() > HELPER_SILENCE;
        }
        if helper_failed {
            if let Some(mut h) = helper.take() {
                let _ = h.child.kill();
                let _ = h.child.wait();
            }
        }
        if helper.is_none() || force_script {
            let (state, players) = poll_script(preferred.as_deref());
            scripted = state.as_ref().map(|s| s.track.player.clone());
            emit.push_players(players);
            emit.push(state);
        }

        let use_helper = helper.is_some() && !force_script;
        let Some(msg) = wait(&rx, if use_helper { POLL } else { SCRIPT_POLL }) else {
            if let Some(mut h) = helper {
                let _ = h.child.kill();
                let _ = h.child.wait();
            }
            return;
        };
        let Some(msg) = msg else { continue };
        match msg {
            Msg::Select(id) => preferred = id,
            Msg::Cmd(cmd) => match (helper.as_ref().filter(|_| use_helper), &scripted) {
                (Some(h), _) => send_helper(&h.dylib, cmd, &mut emit),
                (None, Some(player)) => send_script(player, cmd, &mut emit),
                (None, None) => {}
            },
        }
    }
}

fn send_helper(dylib: &PathBuf, cmd: Command, emit: &mut Emitter) {
    let mut proc = Process::new("/usr/bin/perl");
    proc.args(["-e", PERL_LOADER])
        .arg(dylib)
        .arg("lyrics_island_send")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd {
        Command::PlayPause => {
            proc.env("LYRICS_ISLAND_CMD", "play_pause");
        }
        Command::Next => {
            proc.env("LYRICS_ISLAND_CMD", "next");
        }
        Command::Previous => {
            proc.env("LYRICS_ISLAND_CMD", "previous");
        }
        Command::Seek(ms) => {
            proc.env("LYRICS_ISLAND_CMD", "seek").env("LYRICS_ISLAND_POS_MS", ms.to_string());
            emit.push_seek(ms);
        }
    }
    let _ = proc.status();
}

fn send_script(player: &str, cmd: Command, emit: &mut Emitter) {
    // `player` is one of our two literal names, never user input.
    if !matches!(player, "Spotify" | "Music") {
        return;
    }
    let script = match cmd {
        Command::PlayPause => format!(r#"const a = Application("{player}"); if (a.running()) a.playpause();"#),
        Command::Next => format!(r#"const a = Application("{player}"); if (a.running()) a.nextTrack();"#),
        Command::Previous => format!(r#"const a = Application("{player}"); if (a.running()) a.previousTrack();"#),
        Command::Seek(ms) => {
            emit.push_seek(ms);
            let sec = ms as f64 / 1000.0;
            format!(r#"const a = Application("{player}"); if (a.running()) a.playerPosition = {sec};"#)
        }
    };
    let _ = osascript(&script, None);
}

fn find_dylib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("libmr_helper.dylib"), dir.join("../Frameworks/libmr_helper.dylib")]
        .into_iter()
        .find(|p| p.is_file())
}

fn spawn_helper(dylib: PathBuf) -> Option<Helper> {
    let mut child = Process::new("/usr/bin/perl")
        .args(["-e", PERL_LOADER])
        .arg(&dylib)
        .arg("lyrics_island_stream")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, states) = mpsc::channel();
    std::thread::Builder::new()
        .name("mr-helper".into())
        .spawn(move || {
            let mut art: Option<ArtRef> = None;
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(mut info) = serde_json::from_str::<Info>(&line) else { continue };
                match info.artwork_b64.take().as_deref() {
                    Some("") => art = None,
                    Some(b64) => {
                        art = base64::engine::general_purpose::STANDARD.decode(b64).ok().map(|b| ArtRef::Bytes(Arc::new(b)))
                    }
                    None => {}
                }
                // Parsed here so `at` is stamped when the line arrives, not when the media thread wakes up.
                if tx.send(info.into_state(art.clone())).is_err() {
                    break;
                }
            }
        })
        .ok()?;
    Some(Helper { dylib, child, states, started: Instant::now(), heard: false })
}

fn poll_script(preferred: Option<&str>) -> (Option<PlayerState>, Vec<PlayerChoice>) {
    let Some(out) = osascript(POLL_SCRIPT, preferred) else { return (None, Vec::new()) };
    let Ok(res) = serde_json::from_str::<ScriptPoll>(out.trim()) else { return (None, Vec::new()) };
    let choices = res
        .players
        .into_iter()
        .map(|p| PlayerChoice { id: p.clone(), name: p })
        .collect();
    let state = res.info.and_then(|info| {
        let art = (!info.art_url.is_empty()).then(|| ArtRef::Url(info.art_url.clone()));
        info.into_state(art)
    });
    (state, choices)
}

fn osascript(script: &str, player_env: Option<&str>) -> Option<String> {
    let mut proc = Process::new("/usr/bin/osascript");
    proc.args(["-l", "JavaScript", "-e", script]).stdin(Stdio::null()).stderr(Stdio::null());
    if let Some(p) = player_env {
        proc.env("LYRICS_ISLAND_PLAYER", p);
    }
    let out = proc.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_helper_and_script_lines() {
        let info: Info = serde_json::from_str(r#"{"title":"T","artist":"A","album":"B","duration_ms":200000,"elapsed_ms":1500,"rate":0}"#).unwrap();
        let s = info.into_state(None).unwrap();
        assert_eq!((s.track.player.as_str(), s.status, s.position_ms, s.rate), ("MediaRemote", Status::Paused, 1500, 1.0));
        assert!(serde_json::from_str::<Info>("{}").unwrap().into_state(None).is_none());
        let info: Info = serde_json::from_str(r#"{"player":"Music","title":"T","duration_ms":201500.5,"elapsed_ms":3.25,"rate":1}"#).unwrap();
        let s = info.into_state(None).unwrap();
        assert_eq!((s.track.length_ms, s.status), (201500, Status::Playing));
    }
}
