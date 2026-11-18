# Lyrics Island desktop

A small native app for Windows, macOS and Linux that shows time-synced lyrics for whatever is playing. It ports the GNOME Shell extension in the repository root to a floating window:

- **Island:** a pill showing the current line, configurable in maximum width and font size. Click it to expand: album art, title, artist, the previous, current and next lines, a progress bar, previous/play/next buttons and the lyrics source. Click or drag the progress bar to seek, or click the previous or next lyric line to jump to it. While expanded, `Tab` moves focus, `Enter` or `Space` activates the focused control, `Left` and `Right` seek backward or forward, and `Escape` collapses the card. Clicking elsewhere also collapses it.
- **Desktop lyrics:** a large transparent overlay with the current line, off by default.
- **Themes:** light, dark or system-following (`system`, `dark`, `light`) for the island and settings window.
- **Karaoke highlight:** words light up as they are sung when the lyrics have word timings.
- **Lyrics:** a local `.lrc` file if there is one, otherwise lrc.red, Better Lyrics, LRCLIB, KuGou, Netease and QQ Music, in that order. Only synced lyrics are used. Online results are cached in the user cache folder under `lyrics-island/desktop/`.

Both overlay windows can be dragged; their positions are saved. In click-through mode they ignore the mouse, so use the tray menu or a hotkey to turn it off again.

## Install

Download the build for your system from the `Desktop app` workflow artifacts on GitHub:

- **Linux:** `lyrics-island_0.1.0_amd64.deb`, `lyrics-island-linux-x86_64.tar.gz` or the standalone `lyrics-island` binary.
- **Windows:** `lyrics-island.exe`.
- **macOS:** `Lyrics Island.app.zip`. The macOS app is not notarized: right-click it and choose **Open** the first time.

On Debian or Ubuntu, install the `.deb` package:

```sh
sudo dpkg -i lyrics-island_0.1.0_amd64.deb
```

From the unpacked Linux tarball or from `desktop/` in a source checkout, `install.sh` installs the binary to `${PREFIX:-$HOME/.local}/bin/lyrics-island` together with the desktop entry and icon under `${XDG_DATA_HOME:-$HOME/.local/share}`:

```sh
./install.sh
# Remove the installed files:
./install.sh --uninstall
```

To build from source you need Rust (stable). On Linux also install `pkg-config`, `libssl-dev`, `libx11-dev` and `libxcb1-dev` (or your distribution's equivalents). `./install.sh` runs `cargo build --release` automatically when `target/release/lyrics-island` is missing, or you can build directly:

```sh
cd desktop
cargo build --release
# target/release/lyrics-island (.exe on Windows)
```

On macOS, `target/release/libmr_helper.dylib` must sit in `Lyrics Island.app/Contents/Frameworks/`; see `.github/workflows/desktop.yml` for the full bundle layout.

## Settings and config

Choose **Settings...** in the tray menu to configure appearance, lyric timing, media source, launch at login, local `.lrc` folders and global shortcuts (including an interactive shortcut recorder).

Settings are stored as `config.json` in:

| System | Folder |
|---|---|
| Linux | `~/.config/lyrics-island/` |
| macOS | `~/Library/Application Support/lyrics-island/` |
| Windows | `%APPDATA%\lyrics-island\` |

The file is created with defaults on first start, and **Open config file** in the tray menu opens it in your text editor. External edits to `config.json` are reloaded automatically while the app is running. At startup, a file that isn't valid JSON is renamed to `config.json.bak` and replaced with defaults.

| Key | Default | Meaning |
|---|---|---|
| `ui.island_visible` | `true` | Show the island |
| `ui.desktop_visible` | `false` | Show the desktop lyrics overlay |
| `ui.click_through` | `false` | Both overlays ignore the mouse |
| `ui.theme` | `"system"` | Colour theme for the island and settings window: `"system"`, `"dark"` or `"light"` |
| `ui.island_max_width` | `850` | Maximum width of the collapsed island in logical pixels (200 to 1400) |
| `ui.island_font_pt` | `11` | Font size of the collapsed island lyric line in points (8 to 18) |
| `ui.desktop_font_px` | `34` | Font size of the desktop lyrics in pixels (16 to 96) |
| `ui.offset_ms` | `0` | Added to the playback position in ms (−30000 to 30000). Positive values show lines earlier |
| `ui.karaoke` | `true` | Word-by-word highlight |
| `ui.island_pos`, `ui.desktop_pos` | `null` | Window position as `[x, y]` in physical pixels; `null` = island top centre, desktop lyrics bottom centre |
| `preferred_player` | `null` | Pinned player ID (MPRIS bus name, GSMTC app ID or macOS bundle ID); `null` picks automatically |
| `lrc_dirs` | `[]` | Folders searched for `.lrc` files (see below) |
| `hotkeys.*` | see below | Key combinations; an empty string disables one |

## Local lyrics

For a local audio file the app first looks for an `.lrc` file with the same name next to it. Then it looks in each `lrc_dirs` folder (not subfolders) for `Artist - Title.lrc`, then `Title.lrc`. Case is ignored and suffixes such as `(Official Video)` are stripped from the title first. Enhanced LRC word tags (`<01:02.34>`) enable the karaoke highlight.

## Hotkeys

| Key | Default | Action |
|---|---|---|
| `toggle_island` | `Ctrl+Alt+L` | Show or hide the island |
| `toggle_desktop` | `Ctrl+Alt+D` | Show or hide desktop lyrics |
| `toggle_click_through` | `Ctrl+Alt+T` | Toggle click-through |
| `play_pause` | `Ctrl+Alt+P` | Play or pause |
| `offset_earlier` | `Ctrl+Alt+Left` | Show lines 250 ms earlier (offset +250 ms) |
| `offset_later` | `Ctrl+Alt+Right` | Show lines 250 ms later (offset −250 ms) |
| `offset_reset` | `Ctrl+Alt+0` | Offset back to 0 |

Shortcuts can be recorded interactively in **Settings...** or edited in `config.json`, and changes apply immediately without restarting. Modifiers are `Ctrl`, `Alt` (`Option`), `Shift` and `Super` (`Cmd`); keys are names such as `A`, `0`, `F5`, `Space`, `Left`. Non-function keys require at least one modifier. A combination that doesn't parse or is already taken by another app is skipped.

## Tray

The tray (menu bar on macOS) icon menu has: Settings..., Show island, Desktop lyrics, Click-through, Karaoke highlight, Theme (System, Dark, Light), Player (automatic selection or a specific active player), Lyrics offset (earlier, later, Reset, and the current value), Reset window positions, Launch at login, Open config file and Quit.

## Platform notes

- **Linux:** media comes from event-driven MPRIS (`PropertiesChanged` and `Seeked` D-Bus signals). On Wayland sessions the app runs through XWayland by default so overlay windows can be placed anywhere on screen, and falls back to native Wayland (`wl_shm`) when X11 is absent or `LYRICS_ISLAND_BACKEND=wayland` is set. On Wayland sessions, global shortcuts also register with `org.freedesktop.portal.GlobalShortcuts` so hotkeys work while native Wayland windows have focus. The tray icon needs a StatusNotifier host: KDE and most panels have one; on GNOME install the AppIndicator extension.
- **macOS:** media comes from MediaRemote through a small helper library loaded into the system `perl` (macOS 15.4 and later only let Apple-signed processes read it). If that fails, Spotify and Music are read through AppleScript, and macOS asks for permission the first time. The app has no Dock icon.
- **Windows:** media comes from the system media controls (GSMTC), so any app that shows up in the volume flyout works.

Errors go to stderr: start the app from a terminal on Linux or macOS to see them. The Windows build has no console.
