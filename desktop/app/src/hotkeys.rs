use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use winit::event_loop::EventLoopProxy;
use winit::keyboard::{KeyCode, ModifiersState};

use crate::config;
use crate::{Action, AppEvent};

type SharedHandler = Arc<Mutex<Option<Box<dyn Fn(u32) + Send + 'static>>>>;

static HANDLER: OnceLock<SharedHandler> = OnceLock::new();

/// Keeps the hotkey manager (and on Wayland, the XDG GlobalShortcuts portal session) alive.
pub struct Hotkeys {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    actions: Arc<Mutex<HashMap<u32, Action>>>,
    #[cfg(target_os = "linux")]
    portal_tx: Option<std::sync::mpsc::Sender<config::Hotkeys>>,
}

impl Hotkeys {
    /// Must run on the main thread with the event loop running (macOS/Windows requirement).
    pub fn new(keys: &config::Hotkeys, proxy: EventLoopProxy<AppEvent>) -> (Hotkeys, Vec<(usize, String)>) {
        let actions: Arc<Mutex<HashMap<u32, Action>>> = Arc::new(Mutex::new(HashMap::new()));
        let cell = HANDLER.get_or_init(|| {
            let shared: SharedHandler = Arc::new(Mutex::new(None));
            let s = shared.clone();
            GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
                if e.state == HotKeyState::Pressed {
                    if let Some(cb) = s.lock().unwrap().as_ref() {
                        cb(e.id);
                    }
                }
            }));
            shared
        });
        {
            let map = actions.clone();
            let p = proxy.clone();
            *cell.lock().unwrap() = Some(Box::new(move |id| {
                if let Some(action) = map.lock().unwrap().get(&id).cloned() {
                    let _ = p.send_event(AppEvent::Action(action));
                }
            }));
        }

        // On pure Wayland without XWayland, GlobalHotKeyManager (X11) fails; portal handles it instead.
        let manager = GlobalHotKeyManager::new()
            .map_err(|e| eprintln!("lyrics-island: X11/OS global hotkeys unavailable: {e}"))
            .ok();

        #[cfg(target_os = "linux")]
        let portal_tx = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            wayland_portal::spawn(keys.clone(), proxy)
        } else {
            None
        };

        let mut hk = Hotkeys {
            manager,
            registered: Vec::new(),
            actions,
            #[cfg(target_os = "linux")]
            portal_tx,
        };
        let errs = hk.rebind(keys);
        (hk, errs)
    }

    /// Replaces active bindings at runtime; returns `(binding_index, reason)` for any that failed.
    pub fn rebind(&mut self, keys: &config::Hotkeys) -> Vec<(usize, String)> {
        let mut errs = Vec::new();
        if let Some(m) = &self.manager {
            if !self.registered.is_empty() {
                let _ = m.unregister_all(&self.registered);
                self.registered.clear();
            }
            let mut map = self.actions.lock().unwrap();
            map.clear();
            for (idx, (accel, action)) in keys.bindings().into_iter().enumerate() {
                let trimmed = accel.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let hotkey: HotKey = match trimmed.parse() {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("lyrics-island: invalid hotkey {trimmed:?}: {e}");
                        errs.push((idx, "Invalid shortcut syntax".into()));
                        continue;
                    }
                };
                match m.register(hotkey) {
                    Ok(()) => {
                        self.registered.push(hotkey);
                        map.insert(hotkey.id(), action);
                    }
                    Err(e) => {
                        eprintln!("lyrics-island: can't register hotkey {trimmed:?}: {e}");
                        errs.push((idx, "Shortcut already in use or unavailable".into()));
                    }
                }
            }
        } else {
            // Still validate syntax when running portal-only so Settings shows honest errors.
            for (idx, (accel, _)) in keys.bindings().into_iter().enumerate() {
                let trimmed = accel.trim();
                if !trimmed.is_empty() && trimmed.parse::<HotKey>().is_err() {
                    errs.push((idx, "Invalid shortcut syntax".into()));
                }
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(tx) = &self.portal_tx {
            let _ = tx.send(keys.clone());
        }
        errs
    }
}

/// Converts a pressed key + modifiers from winit into canonical `Ctrl+Alt+Key` config syntax.
/// Returns `None` for bare modifier presses or unsupported keys.
pub fn format_shortcut(mods: ModifiersState, code: KeyCode) -> Option<String> {
    let key = key_name(code)?;
    let mut parts = Vec::with_capacity(5);
    if mods.control_key() {
        parts.push("Ctrl");
    }
    if mods.alt_key() {
        parts.push("Alt");
    }
    if mods.shift_key() {
        parts.push("Shift");
    }
    if mods.super_key() {
        parts.push("Super");
    }
    // Require at least one modifier on non-function keys so plain typing keys aren't grabbed globally.
    if parts.is_empty() && !key.starts_with('F') {
        return None;
    }
    parts.push(key);
    let out = parts.join("+");
    out.parse::<HotKey>().ok()?;
    Some(out)
}

fn key_name(code: KeyCode) -> Option<&'static str> {
    Some(match code {
        KeyCode::KeyA => "A",
        KeyCode::KeyB => "B",
        KeyCode::KeyC => "C",
        KeyCode::KeyD => "D",
        KeyCode::KeyE => "E",
        KeyCode::KeyF => "F",
        KeyCode::KeyG => "G",
        KeyCode::KeyH => "H",
        KeyCode::KeyI => "I",
        KeyCode::KeyJ => "J",
        KeyCode::KeyK => "K",
        KeyCode::KeyL => "L",
        KeyCode::KeyM => "M",
        KeyCode::KeyN => "N",
        KeyCode::KeyO => "O",
        KeyCode::KeyP => "P",
        KeyCode::KeyQ => "Q",
        KeyCode::KeyR => "R",
        KeyCode::KeyS => "S",
        KeyCode::KeyT => "T",
        KeyCode::KeyU => "U",
        KeyCode::KeyV => "V",
        KeyCode::KeyW => "W",
        KeyCode::KeyX => "X",
        KeyCode::KeyY => "Y",
        KeyCode::KeyZ => "Z",
        KeyCode::Digit0 => "0",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6",
        KeyCode::Digit7 => "7",
        KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        KeyCode::Space => "Space",
        KeyCode::Enter => "Enter",
        KeyCode::Tab => "Tab",
        KeyCode::ArrowUp => "Up",
        KeyCode::ArrowDown => "Down",
        KeyCode::ArrowLeft => "Left",
        KeyCode::ArrowRight => "Right",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        KeyCode::Minus => "Minus",
        KeyCode::Equal => "Equal",
        KeyCode::BracketLeft => "BracketLeft",
        KeyCode::BracketRight => "BracketRight",
        KeyCode::Semicolon => "Semicolon",
        KeyCode::Quote => "Quote",
        KeyCode::Comma => "Comma",
        KeyCode::Period => "Period",
        KeyCode::Slash => "Slash",
        KeyCode::Backquote => "Backquote",
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        _ => return None,
    })
}

/// Converts `"Ctrl+Alt+L"` into XDG GlobalShortcuts trigger syntax (`"<Control><Alt>l"`).
#[cfg(any(target_os = "linux", test))]
fn to_xdg_trigger(accel: &str) -> Option<String> {
    let hk: HotKey = accel.trim().parse().ok()?;
    let mut out = String::new();
    use global_hotkey::hotkey::{Code, Modifiers};
    if hk.mods.contains(Modifiers::CONTROL) {
        out.push_str("<Control>");
    }
    if hk.mods.contains(Modifiers::ALT) {
        out.push_str("<Alt>");
    }
    if hk.mods.contains(Modifiers::SHIFT) {
        out.push_str("<Shift>");
    }
    if hk.mods.contains(Modifiers::META) {
        out.push_str("<Super>");
    }
    let key = match hk.key {
        Code::KeyA => "a",
        Code::KeyB => "b",
        Code::KeyC => "c",
        Code::KeyD => "d",
        Code::KeyE => "e",
        Code::KeyF => "f",
        Code::KeyG => "g",
        Code::KeyH => "h",
        Code::KeyI => "i",
        Code::KeyJ => "j",
        Code::KeyK => "k",
        Code::KeyL => "l",
        Code::KeyM => "m",
        Code::KeyN => "n",
        Code::KeyO => "o",
        Code::KeyP => "p",
        Code::KeyQ => "q",
        Code::KeyR => "r",
        Code::KeyS => "s",
        Code::KeyT => "t",
        Code::KeyU => "u",
        Code::KeyV => "v",
        Code::KeyW => "w",
        Code::KeyX => "x",
        Code::KeyY => "y",
        Code::KeyZ => "z",
        Code::Digit0 => "0",
        Code::Digit1 => "1",
        Code::Digit2 => "2",
        Code::Digit3 => "3",
        Code::Digit4 => "4",
        Code::Digit5 => "5",
        Code::Digit6 => "6",
        Code::Digit7 => "7",
        Code::Digit8 => "8",
        Code::Digit9 => "9",
        Code::Space => "space",
        Code::Enter => "Return",
        Code::ArrowUp => "Up",
        Code::ArrowDown => "Down",
        Code::ArrowLeft => "Left",
        Code::ArrowRight => "Right",
        Code::F1 => "F1",
        Code::F2 => "F2",
        Code::F3 => "F3",
        Code::F4 => "F4",
        Code::F5 => "F5",
        Code::F6 => "F6",
        Code::F7 => "F7",
        Code::F8 => "F8",
        Code::F9 => "F9",
        Code::F10 => "F10",
        Code::F11 => "F11",
        Code::F12 => "F12",
        _ => return None,
    };
    out.push_str(key);
    Some(out)
}

#[cfg(target_os = "linux")]
mod wayland_portal {
    use std::collections::HashMap;
    use std::sync::mpsc::{self, Sender};
    use std::time::Duration;

    use winit::event_loop::EventLoopProxy;
    use zbus::blocking::{connection, Connection, MessageIterator};
    use zbus::message::Type;
    use zbus::zvariant::{ObjectPath, OwnedValue, Value};
    use zbus::MatchRule;

    use super::to_xdg_trigger;
    use crate::config;
    use crate::{Action, AppEvent};

    const DEST: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";
    const IFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

    const LABELS: [&str; 7] = [
        "Show or hide island",
        "Show or hide desktop lyrics",
        "Toggle click-through",
        "Play or pause",
        "Show lyrics 250 ms earlier",
        "Show lyrics 250 ms later",
        "Reset lyrics offset",
    ];

    pub fn spawn(initial: config::Hotkeys, proxy: EventLoopProxy<AppEvent>) -> Option<Sender<config::Hotkeys>> {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("wayland-shortcuts".into())
            .spawn(move || {
                let _ = run(initial, rx, proxy);
            })
            .ok()?;
        Some(tx)
    }

    fn run(
        initial: config::Hotkeys,
        rx: mpsc::Receiver<config::Hotkeys>,
        proxy: EventLoopProxy<AppEvent>,
    ) -> zbus::Result<()> {
        let conn = connection::Builder::session()?.method_timeout(Duration::from_secs(3)).build()?;
        let resp_rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface("org.freedesktop.portal.Request")?
            .member("Response")?
            .build();
        let mut resp_iter = MessageIterator::for_match_rule(resp_rule, &conn, Some(16))?;

        let act_rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(IFACE)?
            .member("Activated")?
            .build();
        let act_iter = MessageIterator::for_match_rule(act_rule, &conn, Some(32))?;

        let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
        opts.insert("handle_token", Value::from("lyrics_island_create"));
        opts.insert("session_handle_token", Value::from("lyrics_island_session"));
        let reply = conn.call_method(Some(DEST), PATH, Some(IFACE), "CreateSession", &(opts,))?;
        let reply_body = reply.body();
        let req_path: ObjectPath<'_> = reply_body.deserialize()?;

        let session_path = wait_session_handle(&mut resp_iter, req_path.as_str())?;
        let session = ObjectPath::try_from(session_path.as_str())?;
        bind(&conn, &session, &initial, 1);

        // Listen for rebind requests on a helper thread so Activated signals dispatch without delay.
        let conn_rebind = conn.clone();
        let sess_str = session_path.clone();
        std::thread::spawn(move || {
            let mut seq = 2u32;
            while let Ok(keys) = rx.recv() {
                if let Ok(sess) = ObjectPath::try_from(sess_str.as_str()) {
                    bind(&conn_rebind, &sess, &keys, seq);
                    seq += 1;
                }
            }
        });

        let actions: [Action; 7] = initial.bindings().map(|(_, a)| a);
        for msg in act_iter {
            let Ok(msg) = msg else { break };
            let body = msg.body();
            let Ok((sess, id, _ts, _extra)) =
                body.deserialize::<(ObjectPath<'_>, String, u64, HashMap<String, OwnedValue>)>()
            else {
                continue;
            };
            if sess.as_str() == session_path {
                if let Ok(idx) = id.parse::<usize>() {
                    if let Some(action) = actions.get(idx).cloned() {
                        let _ = proxy.send_event(AppEvent::Action(action));
                    }
                }
            }
        }
        Ok(())
    }

    fn wait_session_handle(iter: &mut MessageIterator, expected_req: &str) -> zbus::Result<String> {
        for msg in iter.take(8) {
            let msg = msg?;
            if msg.header().path().map(|p| p.as_str()) != Some(expected_req) {
                continue;
            }
            let (code, results): (u32, HashMap<String, OwnedValue>) = msg.body().deserialize()?;
            if code != 0 {
                return Err(zbus::Error::Failure("portal CreateSession rejected".into()));
            }
            if let Some(v) = results.get("session_handle") {
                let mut inner: &Value<'_> = v;
                while let Value::Value(b) = inner {
                    inner = b;
                }
                match inner {
                    Value::Str(s) => return Ok(s.to_string()),
                    Value::ObjectPath(p) => return Ok(p.to_string()),
                    _ => {}
                }
            }
        }
        Err(zbus::Error::Failure("portal session_handle missing".into()))
    }

    fn bind(conn: &Connection, session: &ObjectPath<'_>, keys: &config::Hotkeys, seq: u32) {
        let mut shortcuts: Vec<(String, HashMap<&str, Value<'_>>)> = Vec::new();
        for (idx, (accel, _)) in keys.bindings().into_iter().enumerate() {
            let Some(trigger) = to_xdg_trigger(accel) else { continue };
            let mut info = HashMap::new();
            info.insert("description", Value::from(LABELS[idx]));
            info.insert("preferred_trigger", Value::from(trigger));
            shortcuts.push((idx.to_string(), info));
        }
        if shortcuts.is_empty() {
            return;
        }
        let token = format!("lyrics_island_bind_{seq}");
        let mut opts: HashMap<&str, Value<'_>> = HashMap::new();
        opts.insert("handle_token", Value::from(token.as_str()));
        let _ = conn.call_method(Some(DEST), PATH, Some(IFACE), "BindShortcuts", &(session, shortcuts, "", opts));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_converts_shortcuts() {
        let mods = ModifiersState::CONTROL | ModifiersState::ALT;
        let s = format_shortcut(mods, KeyCode::KeyL).unwrap();
        assert_eq!(s, "Ctrl+Alt+L");
        assert_eq!(to_xdg_trigger(&s).as_deref(), Some("<Control><Alt>l"));

        assert_eq!(format_shortcut(ModifiersState::empty(), KeyCode::KeyA), None);
        assert_eq!(format_shortcut(ModifiersState::empty(), KeyCode::F5).as_deref(), Some("F5"));
        assert_eq!(to_xdg_trigger("Ctrl+Alt+Left").as_deref(), Some("<Control><Alt>Left"));
        assert_eq!(to_xdg_trigger("Super+Shift+0").as_deref(), Some("<Shift><Super>0"));
    }
}
