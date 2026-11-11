# Lyrics Island

GNOME Shell extension: a Dynamic Island-style pill in the top bar that shows time-synced lyrics for whatever is playing.

- Works with any MPRIS media player; YouTube Music ([Pear Desktop](https://github.com/pear-devs/pear-desktop)) is preferred when several players are running.
- Synced lyrics are fetched from `api.betterlyrics.org` and `lrc.red`.
- When a track has no synced lyrics, the island shows its title and artist instead.
- Play/pause, next and previous controls on the island.
- Light, dark or system-following colour theme.

Tested on GNOME Shell 46 (X11). `metadata.json` also declares 45 and 47.

## Install

1. Clone into the extensions folder. The folder name must be the extension's uuid:

   ```sh
   git clone git@github.com:yoelngl/lyrics-island.git \
     ~/.local/share/gnome-shell/extensions/lyrics-island@b-log
   ```

2. Restart GNOME Shell so it finds the new extension.
   - X11: press `Alt+F2`, type `r`, press Enter.
   - Wayland: log out and back in.

3. Enable it:

   ```sh
   gnome-extensions enable lyrics-island@b-log
   ```

Once enabled it starts with every login.

## Using it

Start playback in a media player. The island appears in the top bar and follows the current line of the song.

## Settings

Open the preferences window:

```sh
gnome-extensions prefs lyrics-island@b-log
```

| Setting | Range | Default | Meaning |
|---|---|---|---|
| Lyric offset (ms) | -5000 to 5000 | 0 | Added to the playback position. Positive values show lines earlier |
| Maximum compact width (px) | 200 to 1400 | 850 | Widest the collapsed island may grow |
| Compact lyric font size (pt) | 8 to 18 | 11 | Font size of the lyric line |
| Island colour theme | system / dark / light | system | `system` follows the desktop light/dark style |

The same keys are available from the command line:

```sh
gsettings --schemadir ~/.local/share/gnome-shell/extensions/lyrics-island@b-log/schemas \
  set org.gnome.shell.extensions.lyrics-island lyric-offset 300
```

## Troubleshooting

- **No island:** check that the player exposes MPRIS: `busctl --user list | grep org.mpris.MediaPlayer2`.
- **Lyrics early or late:** adjust the lyric offset.
- **Only title and artist:** neither lyrics source has synced lyrics for that track.
- **Errors:** `journalctl --user -b -o cat | grep lyrics-island`

## Development

`tools/fake-player.js` is a fake MPRIS player for testing without a real app:

```sh
gjs -m tools/fake-player.js
```

After changing the settings schema, recompile it:

```sh
glib-compile-schemas schemas/
```

| File | Purpose |
|---|---|
| `extension.js` | Wires player, lyrics and island together; line timing |
| `island.js` | The island widget |
| `lyrics.js` | Lyrics lookup and parsing |
| `mpris.js` | MPRIS player discovery and state |
| `prefs.js` | Preferences window |
| `schemas/` | Settings schema |
