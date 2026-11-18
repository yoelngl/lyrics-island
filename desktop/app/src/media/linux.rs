//! MPRIS over the session bus: event-driven via PropertiesChanged, NameOwnerChanged and Seeked signals,
//! with a periodic resync fallback for players that update Position silently.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use zbus::blocking::{connection, Connection, MessageIterator};
use zbus::message::Type;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};
use zbus::MatchRule;

use super::{wait, Emitter, Msg};
use crate::model::{ArtRef, Command, PlayerChoice, PlayerState, Status, Track};

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";
const IFACE: &str = "org.mpris.MediaPlayer2.Player";
/// Players we'd rather show when several are equally active and none is pinned (same list as mpris.js).
const PREFERRED: [&str; 3] = ["youtube", "pear", "limusic"];
/// Some players (LiMusic) refresh Position only every few seconds; backward jumps smaller than this during polls are stale data.
/// Real backward seeks arrive via the `Seeked` signal and bypass this filter.
const STALE_MS: f64 = 5000.0;
const RESYNC_PLAYING: Duration = Duration::from_millis(1000);
const RESYNC_IDLE: Duration = Duration::from_millis(2500);

struct ActiveMeta {
    bus_name: String,
    owner: Option<String>,
    track_id: Option<String>,
}

pub(super) fn run(tx: Sender<Msg>, rx: Receiver<Msg>, mut preferred: Option<String>, mut emit: Emitter) {
    spawn_signal_listener(tx);

    let mut conn: Option<Connection> = None;
    let mut active: Option<ActiveMeta> = None;
    // Players that ever reported a non-zero Position; others report a constant 0 we must not trust.
    let mut position_works = HashSet::new();
    let mut identities: HashMap<String, String> = HashMap::new();

    loop {
        if conn.is_none() {
            conn = connection::Builder::session()
                .map(|b| b.method_timeout(Duration::from_secs(1)))
                .and_then(|b| b.build())
                .ok();
        }
        let state = match conn.as_ref().map(|c| {
            poll(c, &mut active, preferred.as_deref(), &mut position_works, &mut identities, &mut emit)
        }) {
            Some(Ok(s)) => s,
            Some(Err(_)) => {
                // Bus connection lost; reconnect on the next cycle.
                conn = None;
                None
            }
            None => None,
        };
        let playing = state.as_ref().is_some_and(|s| s.status == Status::Playing);
        emit.push(state);

        let timeout = if playing { RESYNC_PLAYING } else { RESYNC_IDLE };
        let Some(msg) = wait(&rx, timeout) else { return };
        let Some(msg) = msg else { continue };
        handle_msg(msg, &conn, &active, &mut preferred, &mut emit);
        // Drain any burst of queued signals before polling once.
        while let Ok(extra) = rx.try_recv() {
            handle_msg(extra, &conn, &active, &mut preferred, &mut emit);
        }
    }
}

fn handle_msg(
    msg: Msg,
    conn: &Option<Connection>,
    active: &Option<ActiveMeta>,
    preferred: &mut Option<String>,
    emit: &mut Emitter,
) {
    match msg {
        Msg::BusWake => {}
        Msg::Select(id) => *preferred = id,
        Msg::Seeked { sender, pos_ms } => {
            if active.as_ref().and_then(|a| a.owner.as_deref()) == Some(sender.as_str()) {
                emit.push_seek(pos_ms);
            }
        }
        Msg::Cmd(cmd) => {
            if let (Some(c), Some(a)) = (conn, active) {
                send_command(c, a, cmd, emit);
            }
        }
    }
}

fn send_command(conn: &Connection, active: &ActiveMeta, cmd: Command, emit: &mut Emitter) {
    let dest = Some(active.bus_name.as_str());
    match cmd {
        Command::PlayPause => {
            let _ = conn.call_method(dest, PATH, Some(IFACE), "PlayPause", &());
        }
        Command::Next => {
            let _ = conn.call_method(dest, PATH, Some(IFACE), "Next", &());
        }
        Command::Previous => {
            let _ = conn.call_method(dest, PATH, Some(IFACE), "Previous", &());
        }
        Command::Seek(ms) => {
            let target_us = (ms as i64).saturating_mul(1000);
            let set_ok = active
                .track_id
                .as_deref()
                .and_then(|t| ObjectPath::try_from(t).ok())
                .is_some_and(|tid| {
                    conn.call_method(dest, PATH, Some(IFACE), "SetPosition", &(tid, target_us)).is_ok()
                });
            if !set_ok {
                let cur_ms = emit.last().map_or(0, PlayerState::position_now) as i64;
                let delta_us = (ms as i64 - cur_ms).saturating_mul(1000);
                let _ = conn.call_method(dest, PATH, Some(IFACE), "Seek", &(delta_us,));
            }
            emit.push_seek(ms);
        }
    }
}

fn spawn_signal_listener(tx: Sender<Msg>) {
    let _ = std::thread::Builder::new().name("mpris-signals".into()).spawn(move || {
        loop {
            let Ok(conn) = connection::Builder::session().and_then(|b| b.build()) else {
                std::thread::sleep(Duration::from_secs(3));
                continue;
            };
            // Match signals on /org/mpris/MediaPlayer2 (PropertiesChanged + Seeked) and NameOwnerChanged.
            let mpris_rule = MatchRule::builder().msg_type(Type::Signal).path(PATH).ok().map(|b| b.build());
            let owner_rule = MatchRule::builder()
                .msg_type(Type::Signal)
                .interface("org.freedesktop.DBus")
                .ok()
                .and_then(|b| b.member("NameOwnerChanged").ok())
                .map(|b| b.build());
            if let Some(rule) = owner_rule {
                let _ = conn.call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "AddMatch",
                    &(rule.to_string(),),
                );
            }
            let Some(rule) = mpris_rule else { return };
            let Ok(iter) = MessageIterator::for_match_rule(rule, &conn, Some(64)) else {
                std::thread::sleep(Duration::from_secs(3));
                continue;
            };
            for msg in iter {
                let Ok(msg) = msg else { break };
                let hdr = msg.header();
                let member = hdr.member().map(|m| m.as_str());
                let iface = hdr.interface().map(|i| i.as_str());
                if iface == Some(IFACE) && member == Some("Seeked") {
                    if let (Some(sender), Ok(pos_us)) = (hdr.sender(), msg.body().deserialize::<i64>()) {
                        let pos_ms = (pos_us.max(0) / 1000) as u64;
                        if tx.send(Msg::Seeked { sender: sender.to_string(), pos_ms }).is_err() {
                            return;
                        }
                        continue;
                    }
                }
                if iface == Some("org.freedesktop.DBus") && member == Some("NameOwnerChanged") {
                    let relevant = msg
                        .body()
                        .deserialize::<(String, String, String)>()
                        .is_ok_and(|(name, _, _)| name.starts_with(PREFIX));
                    if !relevant {
                        continue;
                    }
                }
                if tx.send(Msg::BusWake).is_err() {
                    return;
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn poll(
    conn: &Connection,
    active: &mut Option<ActiveMeta>,
    preferred: Option<&str>,
    position_works: &mut HashSet<String>,
    identities: &mut HashMap<String, String>,
    emit: &mut Emitter,
) -> zbus::Result<Option<PlayerState>> {
    let names: Vec<String> = conn
        .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "ListNames", &())?
        .body()
        .deserialize()?;
    position_works.retain(|n| names.contains(n));
    identities.retain(|n, _| names.contains(n));

    struct Candidate {
        score: f64,
        state: PlayerState,
        position_us: Option<i64>,
        owner: Option<String>,
        track_id: Option<String>,
    }

    let mut choices: Vec<PlayerChoice> = Vec::new();
    let mut best: Option<Candidate> = None;

    for name in names.iter().filter(|n| n.starts_with(PREFIX)) {
        // A hung or half-implemented player must not take the others down with it.
        let Ok((props, owner)) = get_all(conn, name) else { continue };
        let (state, position_us, track_id) = parse(name, &props);
        if state.status == Status::Stopped || state.track.title.is_empty() {
            continue;
        }
        let label = identities
            .entry(name.clone())
            .or_insert_with(|| fetch_identity(conn, name).unwrap_or_else(|| pretty_bus_name(name)))
            .clone();
        choices.push(PlayerChoice { id: name.clone(), name: label });

        let lower = name.to_lowercase();
        let mut score = 0.0;
        if preferred == Some(name.as_str()) {
            score += 10.0;
        }
        if state.status == Status::Playing {
            score += 2.0;
        }
        if PREFERRED.iter().any(|p| lower.contains(p)) {
            score += 1.0;
        }
        if active.as_ref().map(|a| a.bus_name.as_str()) == Some(name.as_str()) {
            score += 0.5;
        }
        if best.as_ref().is_none_or(|c| score > c.score) {
            best = Some(Candidate { score, state, position_us, owner, track_id });
        }
    }
    choices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    emit.push_players(choices);

    let Some(Candidate { mut state, position_us, owner, track_id, .. }) = best else {
        *active = None;
        return Ok(None);
    };
    *active = Some(ActiveMeta { bus_name: state.track.player.clone(), owner, track_id });

    let continued = emit.last().filter(|l| l.track == state.track).map(PlayerState::position_now);
    let reported = position_us.map(|us| us.max(0) as u64 / 1000).filter(|&ms| {
        if ms > 0 {
            position_works.insert(state.track.player.clone());
        }
        // Players can briefly report the previous track's position past the new track's end.
        position_works.contains(&state.track.player) && (state.track.length_ms == 0 || ms <= state.track.length_ms + 2000)
    });
    state.position_ms = match (reported, continued) {
        (Some(r), Some(c))
            if state.status == Status::Playing && (r as f64) < c as f64 && (r as f64) > c as f64 - STALE_MS =>
        {
            c
        }
        (Some(r), _) => r,
        (None, Some(c)) => c,
        (None, None) => 0,
    };
    Ok(Some(state))
}

fn get_all(conn: &Connection, name: &str) -> zbus::Result<(HashMap<String, OwnedValue>, Option<String>)> {
    let reply = conn.call_method(Some(name), PATH, Some("org.freedesktop.DBus.Properties"), "GetAll", &(IFACE,))?;
    let owner = reply.header().sender().map(|s| s.to_string());
    let props = reply.body().deserialize()?;
    Ok((props, owner))
}

fn fetch_identity(conn: &Connection, name: &str) -> Option<String> {
    let reply = conn
        .call_method(Some(name), PATH, Some("org.freedesktop.DBus.Properties"), "Get", &(ROOT_IFACE, "Identity"))
        .ok()?;
    let val: OwnedValue = reply.body().deserialize().ok()?;
    string(&val).filter(|s| !s.trim().is_empty())
}

fn pretty_bus_name(bus_name: &str) -> String {
    let raw = bus_name.strip_prefix(PREFIX).unwrap_or(bus_name);
    let base = raw.split('.').next().unwrap_or(raw).replace(['_', '-'], " ");
    let mut chars = base.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => bus_name.to_owned(),
    }
}

fn parse(name: &str, props: &HashMap<String, OwnedValue>) -> (PlayerState, Option<i64>, Option<String>) {
    let status = match props.get("PlaybackStatus").and_then(|v| string(v)).as_deref() {
        Some("Playing") => Status::Playing,
        Some("Paused") => Status::Paused,
        _ => Status::Stopped,
    };
    let rate = props.get("Rate").and_then(|v| number(v)).filter(|r| *r > 0.0).unwrap_or(1.0);
    let position_us = props.get("Position").and_then(|v| number(v)).map(|p| p as i64);

    let mut track = Track { player: name.to_owned(), ..Default::default() };
    let mut track_id = None;
    if let Some(Value::Dict(meta)) = props.get("Metadata").map(|v| inner(v)) {
        for (k, v) in meta.iter() {
            let Value::Str(k) = inner(k) else { continue };
            match &**k {
                "mpris:trackid" => track_id = string(v).filter(|s| !s.is_empty()),
                "xesam:title" => track.title = string(v).unwrap_or_default(),
                "xesam:album" => track.album = string(v).unwrap_or_default(),
                "xesam:artist" => {
                    track.artist = match inner(v) {
                        Value::Array(a) => a.inner().iter().filter_map(string).collect::<Vec<_>>().join(", "),
                        other => string(other).unwrap_or_default(),
                    }
                }
                "mpris:length" => track.length_ms = number(v).map_or(0, |us| us.max(0.0) as u64 / 1000),
                "mpris:artUrl" => track.art = string(v).filter(|u| !u.is_empty()).map(ArtRef::Url),
                "xesam:url" => track.file_url = string(v).filter(|u| u.starts_with("file://")),
                _ => {}
            }
        }
    }
    let state = PlayerState { track, status, position_ms: 0, at: Instant::now(), rate };
    (state, position_us, track_id)
}

/// Strips the variant wrappers that a{sv} values arrive in.
fn inner<'a, 'b>(mut v: &'a Value<'b>) -> &'a Value<'b> {
    while let Value::Value(b) = v {
        v = b;
    }
    v
}

fn string(v: &Value) -> Option<String> {
    match inner(v) {
        Value::Str(s) => Some(s.to_string()),
        Value::ObjectPath(p) => Some(p.to_string()),
        _ => None,
    }
}

fn number(v: &Value) -> Option<f64> {
    Some(match *inner(v) {
        Value::I64(n) => n as f64,
        Value::U64(n) => n as f64,
        Value::I32(n) => n as f64,
        Value::U32(n) => n as f64,
        Value::F64(n) => n,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bus_names() {
        assert_eq!(pretty_bus_name("org.mpris.MediaPlayer2.spotify"), "Spotify");
        assert_eq!(pretty_bus_name("org.mpris.MediaPlayer2.firefox.instance_1_99"), "Firefox");
        assert_eq!(pretty_bus_name("org.mpris.MediaPlayer2.youtube_music"), "Youtube music");
    }
}
