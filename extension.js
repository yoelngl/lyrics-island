import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {Island} from './island.js';
import {LyricsProvider} from './lyrics.js';
import {MprisWatcher} from './mpris.js';

const TICK_MS = 100;
const RESYNC_EVERY_TICKS = 10;

const isCancelled = e => e?.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED);

export default class LyricsIslandExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._provider = new LyricsProvider();
        this._player = null;
        this._trackKey = null;
        this._artUrl = null;
        this._lines = null;
        this._lineIdx = null;
        this._lastStateKey = null;
        this._tickId = 0;
        this._ticks = 0;

        this._island = new Island(this._settings);
        Main.layoutManager.addChrome(this._island, {affectsInputRegion: true, trackFullscreen: true});
        this._island.connect('play-pause', () => this._player?.playPause());
        this._island.connect('next', () => this._player?.next());
        this._island.connect('previous', () => this._player?.previous());

        this._offsetId = this._settings.connect('changed::lyric-offset', () => this._tick(true));

        this._watcher = new MprisWatcher();
        this._watcher.connect('update', () => this._onUpdate());
        this._watcher.connect('position', () => this._tick());
        this._watcher.start();
    }

    disable() {
        this._stopTick();
        this._lyricsCancellable?.cancel();
        this._artCancellable?.cancel();
        this._watcher.destroy();
        this._watcher = null;
        this._settings.disconnect(this._offsetId);
        this._island.destroy();
        this._island = null;
        this._provider.destroy();
        this._provider = null;
        this._settings = null;
        this._player = null;
        Main.panel.statusArea.dateMenu?.container.show();
    }

    _updateVisibility() {
        const player = this._player;
        const clock = Main.panel.statusArea.dateMenu?.container;
        const isClockInCenter = Boolean(clock && clock.get_parent() === Main.panel._centerBox);

        if (!player || player.status === 'Stopped') {
            this._island.hideIsland();
            clock?.show();
            return;
        }

        // Always keep dynamic island visible while player is active
        if (isClockInCenter)
            clock?.hide();
        else
            clock?.show();

        this._island.showIsland();
    }

    _onUpdate() {
        const player = this._watcher.active;
        this._player = player;

        if (!player || player.status === 'Stopped') {
            this._stopTick();
            this._trackKey = null;
            this._updateVisibility();
            return;
        }

        const track = player.track;
        // Album and length arrive late from browsers and change what the lyrics query matches.
        const key = `${player.busName}\n${track.title}\n${track.artist}\n${track.album}\n${Math.round(track.length / 1000)}`;
        if (key !== this._trackKey) {
            this._trackKey = key;
            this._island.setTrack(track);
            this._loadLyrics(track, key);
        }
        if (track.artUrl !== this._artUrl)
            this._loadArt(track.artUrl);

        const playing = player.status === 'Playing';
        this._island.setPlaying(playing);
        if (playing)
            this._startTick();
        else
            this._stopTick();

        this._updateVisibility();
        this._tick(true);
    }

    _loadLyrics(track, key) {
        this._lyricsCancellable?.cancel();
        const cancellable = new Gio.Cancellable();
        this._lyricsCancellable = cancellable;
        this._lines = null;
        this._lineIdx = null;
        this._lastStateKey = null;
        this._island.setCredit(null);

        this._provider.fetch(track, cancellable).then(result => {
            if (key !== this._trackKey)
                return;
            this._lines = result.synced;
            this._island.setCredit(result.synced ? result.source : null);
            this._updateVisibility();
            this._tick(true);
        }).catch(e => {
            if (!isCancelled(e))
                console.warn(`[lyrics-island] lyrics: ${e.message}`);
        });
    }

    _loadArt(url) {
        this._artUrl = url;
        this._artCancellable?.cancel();
        const cancellable = new Gio.Cancellable();
        this._artCancellable = cancellable;
        this._island.setArt(null);
        this._provider.fetchArt(url, cancellable).then(uri => {
            if (url === this._artUrl)
                this._island.setArt(uri);
        }).catch(e => {
            if (!isCancelled(e))
                console.warn(`[lyrics-island] art: ${e.message}`);
        });
    }

    _startTick() {
        if (this._tickId)
            return;
        this._tickId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, TICK_MS, () => {
            if (++this._ticks % RESYNC_EVERY_TICKS === 0)
                this._player?.refreshPosition();
            this._tick();
            return GLib.SOURCE_CONTINUE;
        });
    }

    _stopTick() {
        if (this._tickId)
            GLib.source_remove(this._tickId);
        this._tickId = 0;
    }

    _tick(force = false) {
        const player = this._player;
        if (!player || !this._island)
            return;
        const {title, artist, album, length} = player.track;
        const pos = player.position();
        this._island.setProgress(pos, length);

        const lines = this._lines;
        const fallback = artist ? `${title} · ${artist}` : title;

        if (!lines || lines.length === 0) {
            // No synced lyrics: show full title and artist cleanly, fully visible
            if (force || this._lastStateKey !== fallback) {
                this._lastStateKey = fallback;
                this._lineIdx = -1;
                this._island.setLines(fallback, '', fallback, '');
            }
            return;
        }

        const t = pos + this._settings.get_int('lyric-offset');
        let lo = 0, hi = lines.length - 1, idx = -1;
        while (lo <= hi) {
            const mid = (lo + hi) >> 1;
            if (lines[mid].t <= t) {
                idx = mid;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }

        const curLine = idx >= 0 ? lines[idx] : null;
        const nextLine = (idx >= 0 && idx + 1 < lines.length) ? lines[idx + 1] : null;

        // Line stays active until next line or max 9s (during instrumental pauses/solos)
        const maxDuration = nextLine ? Math.min(nextLine.t - curLine.t, 9000) : 8000;
        const isLineActive = curLine && Boolean(curLine.text) && (t >= curLine.t) && (t < curLine.t + maxDuration);

        const compactText = isLineActive ? curLine.text : fallback;
        const cur = isLineActive ? curLine.text : (curLine?.text || fallback);
        const prev = idx > 0 ? (lines[idx - 1].text || '') : '';
        const next = (idx + 1 < lines.length) ? (lines[idx + 1].text || '') : '';

        const stateKey = `${compactText}\n${cur}\n${prev}\n${next}`;
        if (!force && stateKey === this._lastStateKey)
            return;
        this._lastStateKey = stateKey;
        this._lineIdx = idx;

        this._island.setLines(compactText, prev, cur, next);
    }
}
