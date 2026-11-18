//! Launch-at-login registration per OS: XDG autostart (.desktop), macOS LaunchAgents (.plist), Windows Run key.

#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::fs;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::io::ErrorKind;
use std::path::Path;
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use std::path::PathBuf;

pub fn is_enabled() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux_path().is_some_and(|p| is_desktop_enabled(&p))
    }
    #[cfg(target_os = "macos")]
    {
        macos_path().is_some_and(|p| p.is_file())
    }
    #[cfg(windows)]
    {
        windows_query()
    }
}

pub fn set_enabled(enable: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't locate executable: {e}"))?;
    #[cfg(target_os = "linux")]
    {
        let path = linux_path().ok_or_else(|| "config folder unavailable".to_owned())?;
        write_or_remove(&path, enable, || desktop_entry(&exe))
    }
    #[cfg(target_os = "macos")]
    {
        let path = macos_path().ok_or_else(|| "home folder unavailable".to_owned())?;
        write_or_remove(&path, enable, || launch_agent_plist(&exe))
    }
    #[cfg(windows)]
    {
        windows_set(enable, &exe)
    }
}

#[cfg(target_os = "linux")]
fn linux_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("autostart").join("lyrics-island.desktop"))
}

#[cfg(target_os = "macos")]
fn macos_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library/LaunchAgents/dev.blog.lyrics-island.plist"))
}

#[cfg(any(target_os = "linux", target_os = "macos", test))]
fn write_or_remove(path: &Path, enable: bool, content: impl FnOnce() -> String) -> Result<(), String> {
    if enable {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
        }
        fs::write(path, content()).map_err(|e| format!("can't write {}: {e}", path.display()))
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("can't remove {}: {e}", path.display())),
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn is_desktop_enabled(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else { return false };
    !text.lines().any(|l| {
        let l = l.trim();
        l.eq_ignore_ascii_case("Hidden=true") || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

#[cfg(any(target_os = "linux", test))]
fn desktop_entry(exe: &Path) -> String {
    let escaped = exe.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Lyrics Island\n\
         Comment=Time-synced lyrics overlay\n\
         Exec=\"{escaped}\"\n\
         Icon=lyrics-island\n\
         Terminal=false\n\
         Categories=AudioVideo;Audio;\n\
         StartupNotify=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(any(target_os = "macos", test))]
fn launch_agent_plist(exe: &Path) -> String {
    let mut escaped = String::new();
    for c in exe.to_string_lossy().chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
           <key>Label</key><string>dev.blog.lyrics-island</string>\n\
           <key>ProgramArguments</key><array><string>{escaped}</string></array>\n\
           <key>RunAtLoad</key><true/>\n\
         </dict>\n\
         </plist>\n"
    )
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "LyricsIsland";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn windows_query() -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg.exe")
        .args(["query", RUN_KEY, "/v", RUN_VALUE])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

#[cfg(windows)]
fn windows_set(enable: bool, exe: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("reg.exe");
    cmd.creation_flags(CREATE_NO_WINDOW);
    if enable {
        let quoted = format!("\"{}\"", exe.display());
        cmd.args(["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &quoted, "/f"]);
    } else {
        if !windows_query() {
            return Ok(());
        }
        cmd.args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]);
    }
    let out = cmd.output().map_err(|e| format!("reg.exe failed: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_and_plist_round_trip() {
        let dir = std::env::temp_dir().join(format!("lyrics-island-autostart-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let desk = dir.join("autostart/lyrics-island.desktop");
        let exe = PathBuf::from("/opt/Lyrics Island/bin & more/lyrics-island");

        assert!(!is_desktop_enabled(&desk));
        write_or_remove(&desk, true, || desktop_entry(&exe)).unwrap();
        assert!(is_desktop_enabled(&desk));
        let text = fs::read_to_string(&desk).unwrap();
        assert!(text.contains("Exec=\"/opt/Lyrics Island/bin & more/lyrics-island\""));

        fs::write(&desk, "[Desktop Entry]\nX-GNOME-Autostart-enabled=false\n").unwrap();
        assert!(!is_desktop_enabled(&desk));

        write_or_remove(&desk, false, String::new).unwrap();
        assert!(!desk.exists());
        // Removing again is a no-op.
        write_or_remove(&desk, false, String::new).unwrap();

        let plist = launch_agent_plist(&exe);
        assert!(plist.contains("/opt/Lyrics Island/bin &amp; more/lyrics-island"));

        let _ = fs::remove_dir_all(&dir);
    }
}
