//! Single-instance guard: prevents opening a second Lyrics Island process when one is already running.

#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(unix)]
use std::path::PathBuf;

use winit::event_loop::EventLoopProxy;

use crate::AppEvent;
#[cfg(unix)]
use crate::Action;

pub struct InstanceGuard {
    #[cfg(unix)]
    sock_path: PathBuf,
    #[cfg(unix)]
    listener: Option<UnixListener>,
    #[cfg(windows)]
    _lock_file: std::fs::File,
}

impl InstanceGuard {
    /// Must run before building the event loop so duplicate launches exit immediately without touching X11/OS windows.
    pub fn try_lock(open_settings_on_existing: bool) -> Option<Self> {
        #[cfg(unix)]
        {
            try_lock_unix(&sock_path(), open_settings_on_existing)
        }
        #[cfg(windows)]
        {
            let _ = open_settings_on_existing;
            try_lock_windows()
        }
    }

    pub fn spawn_ipc(&mut self, proxy: EventLoopProxy<AppEvent>) {
        #[cfg(unix)]
        if let Some(listener) = self.listener.take() {
            use std::io::Read;
            let _ = std::thread::Builder::new().name("instance-ipc".into()).spawn(move || {
                for conn in listener.incoming().flatten() {
                    let mut buf = String::new();
                    let _ = conn.take(64).read_to_string(&mut buf);
                    if buf.trim() == "settings" {
                        let _ = proxy.send_event(AppEvent::Action(Action::OpenSettings));
                    }
                }
            });
        }
        #[cfg(windows)]
        let _ = proxy;
    }
}

#[cfg(unix)]
fn sock_path() -> PathBuf {
    let base = dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(std::env::temp_dir);
    base.join("lyrics-island-instance.sock")
}

#[cfg(unix)]
fn try_lock_unix(path: &PathBuf, open_settings_on_existing: bool) -> Option<InstanceGuard> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    if let Ok(mut stream) = UnixStream::connect(path) {
        let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
        let msg: &[u8] = if open_settings_on_existing { b"settings\n" } else { b"noop\n" };
        let _ = stream.write_all(msg);
        eprintln!("lyrics-island: already running");
        return None;
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(path);
    match UnixListener::bind(path) {
        Ok(listener) => Some(InstanceGuard { sock_path: path.clone(), listener: Some(listener) }),
        Err(e) => {
            eprintln!("lyrics-island: can't bind instance lock {}: {e}", path.display());
            None
        }
    }
}

#[cfg(unix)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

#[cfg(windows)]
fn try_lock_windows() -> Option<InstanceGuard> {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("lyrics-island");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("instance.lock");
    match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .share_mode(0)
        .open(&path)
    {
        Ok(file) => Some(InstanceGuard { _lock_file: file }),
        Err(_) => {
            eprintln!("lyrics-island: already running");
            None
        }
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    #[test]
    fn blocks_duplicate_instance_and_recovers_stale_socket() {
        let path = std::env::temp_dir().join(format!("li-single-inst-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let first = try_lock_unix(&path, false);
        assert!(first.is_some(), "first instance acquires lock");
        assert!(try_lock_unix(&path, false).is_none(), "second instance rejected while first is alive");
        drop(first);

        let after_drop = try_lock_unix(&path, false);
        assert!(after_drop.is_some(), "new instance acquires lock after first exits");
        drop(after_drop);
    }
}
