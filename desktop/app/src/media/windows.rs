//! Global System Media Transport Controls (the media flyout's source), polled.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ::windows::core::Result;
use ::windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Properties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use ::windows::Storage::Streams::DataReader;
use ::windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use super::{wait, Emitter, Msg, POLL};
use crate::model::{ArtRef, Command, PlayerChoice, PlayerState, Status, Track};

/// Seconds between 1601-01-01 (WinRT DateTime epoch) and 1970-01-01.
const EPOCH_DIFF_S: i64 = 11_644_473_600;

/// Thumbnails are only fetched once per track; keyed by app + title + artist + album.
type ArtCache = Option<((String, String, String, String), Arc<Vec<u8>>)>;

pub(super) fn run(_tx: Sender<Msg>, rx: Receiver<Msg>, mut preferred: Option<String>, mut emit: Emitter) {
    // Ignoring the result: a fresh thread can't already be in a conflicting apartment.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let mut manager: Option<Manager> = None;
    let mut art: ArtCache = None;
    loop {
        if manager.is_none() {
            manager = Manager::RequestAsync().and_then(|op| op.join()).ok();
        }
        let (session, choices) = pick_session(manager.as_ref(), preferred.as_deref());
        emit.push_players(choices);
        emit.push(session.as_ref().and_then(|s| sample(s, &mut art).ok().flatten()));
        match wait(&rx, POLL) {
            None => return,
            Some(Some(Msg::Select(id))) => preferred = id,
            Some(Some(Msg::Cmd(cmd))) => {
                if let Some(s) = &session {
                    match cmd {
                        Command::PlayPause => {
                            let _ = s.TryTogglePlayPauseAsync().and_then(|op| op.join());
                        }
                        Command::Next => {
                            let _ = s.TrySkipNextAsync().and_then(|op| op.join());
                        }
                        Command::Previous => {
                            let _ = s.TrySkipPreviousAsync().and_then(|op| op.join());
                        }
                        Command::Seek(ms) => {
                            let start = s
                                .GetTimelineProperties()
                                .and_then(|t| t.StartTime())
                                .map_or(0, |d| d.Duration);
                            let ticks = start.saturating_add((ms as i64).saturating_mul(10_000));
                            let _ = s.TryChangePlaybackPositionAsync(ticks).and_then(|op| op.join());
                            emit.push_seek(ms);
                        }
                    }
                }
            }
            Some(None) => {}
        }
    }
}

fn pick_session(manager: Option<&Manager>, preferred: Option<&str>) -> (Option<Session>, Vec<PlayerChoice>) {
    let Some(m) = manager else { return (None, Vec::new()) };
    let mut choices = Vec::new();
    let mut pinned = None;
    if let Ok(list) = m.GetSessions() {
        for i in 0..list.Size().unwrap_or(0) {
            let Ok(s) = list.GetAt(i) else { continue };
            let Ok(id) = s.SourceAppUserModelId().map(|h| h.to_string()) else { continue };
            if id.is_empty() {
                continue;
            }
            if preferred == Some(id.as_str()) {
                pinned = Some(s);
            }
            let name = pretty_app_id(&id);
            if !choices.iter().any(|c: &PlayerChoice| c.id == id) {
                choices.push(PlayerChoice { id, name });
            }
        }
    }
    choices.sort_by(|a, b| a.name.cmp(&b.name));
    let active = pinned.or_else(|| m.GetCurrentSession().ok());
    (active, choices)
}

fn pretty_app_id(id: &str) -> String {
    let base = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let stem = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    stem.split('!').next().unwrap_or(stem).to_owned()
}

fn sample(session: &Session, art: &mut ArtCache) -> Result<Option<PlayerState>> {
    let info = session.GetPlaybackInfo()?;
    let status = match info.PlaybackStatus()? {
        PlaybackStatus::Playing => Status::Playing,
        PlaybackStatus::Paused | PlaybackStatus::Changing => Status::Paused,
        PlaybackStatus::Closed => return Ok(None),
        _ => Status::Stopped,
    };
    let rate = info.PlaybackRate().and_then(|r| r.Value()).ok().filter(|r| *r > 0.0).unwrap_or(1.0);
    let props = session.TryGetMediaPropertiesAsync()?.join()?;
    let player = session.SourceAppUserModelId()?.to_string();
    let title = props.Title()?.to_string();
    if title.is_empty() {
        return Ok(None);
    }
    let artist = props.Artist()?.to_string();
    let album = props.AlbumTitle()?.to_string();

    let key = (player.clone(), title.clone(), artist.clone(), album.clone());
    let bytes = match art.as_ref().filter(|(k, _)| *k == key) {
        Some((_, b)) => Some(b.clone()),
        // Apps often publish the thumbnail a moment after the title, so a miss is retried next poll.
        None => read_thumbnail(&props).ok().filter(|b| !b.is_empty()).map(|b| {
            let b = Arc::new(b);
            *art = Some((key, b.clone()));
            b
        }),
    };

    let timeline = session.GetTimelineProperties()?;
    let start = timeline.StartTime()?.Duration;
    let end = timeline.EndTime()?.Duration;
    let mut position = (timeline.Position()?.Duration - start) as f64;
    if status == Status::Playing {
        // Apps update the timeline only on events; extrapolate from when they last did.
        let updated = timeline.LastUpdatedTime()?.UniversalTime;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| (d.as_nanos() / 100) as i64) + EPOCH_DIFF_S * 10_000_000;
        if updated > 0 {
            position += (now - updated).max(0) as f64 * rate;
        }
    }
    let length_ms = ((end - start).max(0) / 10_000) as u64;
    let mut position_ms = (position.max(0.0) / 10_000.0) as u64;
    if length_ms > 0 {
        position_ms = position_ms.min(length_ms);
    }

    Ok(Some(PlayerState {
        track: Track { title, artist, album, length_ms, art: bytes.map(ArtRef::Bytes), file_url: None, player },
        status,
        position_ms,
        at: Instant::now(),
        rate,
    }))
}

fn read_thumbnail(props: &Properties) -> Result<Vec<u8>> {
    let stream = props.Thumbnail()?.OpenReadAsync()?.join()?;
    let size = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream)?;
    let loaded = reader.LoadAsync(size)?.join()?;
    let mut buf = vec![0; loaded as usize];
    reader.ReadBytes(&mut buf)?;
    Ok(buf)
}
