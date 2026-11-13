import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import * as Signals from 'resource:///org/gnome/shell/misc/signals.js';

const MPRIS_PREFIX = 'org.mpris.MediaPlayer2.';
const MPRIS_PATH = '/org/mpris/MediaPlayer2';
const PLAYER_IFACE = 'org.mpris.MediaPlayer2.Player';
const PREFERRED = /youtube|pear|limusic/i;
const STALE_POSITION_MS = 5000;

const PlayerProxy = Gio.DBusProxy.makeProxyWrapper(`
<node>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="PlayPause"/>
    <method name="Next"/>
    <method name="Previous"/>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <signal name="Seeked"><arg type="x" name="position"/></signal>
  </interface>
</node>`);

const nowMs = () => GLib.get_monotonic_time() / 1000;

function parseMetadata(meta) {
    const artist = meta['xesam:artist'];
    return {
        title: meta['xesam:title'] ?? '',
        artist: Array.isArray(artist) ? artist.join(', ') : (artist ?? ''),
        album: meta['xesam:album'] ?? '',
        length: Number(meta['mpris:length'] ?? 0) / 1000,
        artUrl: meta['mpris:artUrl'] ?? '',
    };
}

class Player extends Signals.EventEmitter {
    constructor(busName) {
        super();
        this.busName = busName;
        this.preferred = PREFERRED.test(busName);
        this.ready = false;
        this.status = 'Stopped';
        this.track = parseMetadata({});
        this._posBase = 0;
        this._posTime = nowMs();
        this._cancellable = new Gio.Cancellable();

        this._proxy = new PlayerProxy(Gio.DBus.session, busName, MPRIS_PATH, (proxy, error) => {
            if (error) {
                if (!error.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                    console.warn(`[lyrics-island] proxy ${busName}: ${error.message}`);
                return;
            }
            this._propsId = proxy.connect('g-properties-changed', () => this._sync());
            this._seekedId = proxy.connectSignal('Seeked', (_p, _s, [pos]) => this._setPosition(Number(pos)));
            this.ready = true;
            this._sync();
        }, this._cancellable);
    }

    _sync() {
        const prevStatus = this.status;
        const prevTitle = this.track.title;
        this.status = this._proxy.PlaybackStatus ?? 'Stopped';
        const meta = this._proxy.get_cached_property('Metadata');
        this.track = parseMetadata(meta ? meta.recursiveUnpack() : {});

        if (this.track.title !== prevTitle) {
            this._posBase = 0;
            this._posTime = nowMs();
        }
        if (prevStatus !== this.status) {
            // Freeze the interpolated position at the moment of the state change.
            this._posBase = this.position();
            this._posTime = nowMs();
        }
        if (this.track.title !== prevTitle || prevStatus !== this.status)
            this.refreshPosition();
        this.emit('changed');
    }

    /** Current position in ms, interpolated between D-Bus reads. */
    position() {
        if (this.status !== 'Playing')
            return this._posBase;
        return this._posBase + (nowMs() - this._posTime);
    }

    refreshPosition() {
        Gio.DBus.session.call(this.busName, MPRIS_PATH, 'org.freedesktop.DBus.Properties', 'Get',
            new GLib.Variant('(ss)', [PLAYER_IFACE, 'Position']), new GLib.VariantType('(v)'),
            Gio.DBusCallFlags.NONE, 1000, this._cancellable, (conn, res) => {
                try {
                    const pos = Number(conn.call_finish(res).recursiveUnpack()[0]);
                    // Some players never update Position (always 0), and 1 s resyncs to 0 would pin the bar near 0:00.
                    // A 0 adds nothing over our own track-start reset, so trust it only once the player has reported a real value.
                    if (pos > 0)
                        this._positionWorks = true;
                    else if (!this._positionWorks)
                        return;
                    // Polled values lag: by IPC latency, and some players (LiMusic) only refresh Position every few seconds.
                    // While playing, our interpolation is accurate, so small backward corrections are stale data; real seeks come via Seeked.
                    const delta = pos / 1000 - this.position();
                    if (delta < 300 && delta > (this.status === 'Playing' ? -STALE_POSITION_MS : -300))
                        return;
                    this._setPosition(pos);
                } catch {
                    // Player may not implement Position; keep interpolating.
                }
            });
    }

    _setPosition(us) {
        let posMs = Number(us) / 1000;
        if (isNaN(posMs) || posMs < 0)
            posMs = 0;
        // Ignore stale position values that exceed track duration during track transitions
        if (this.track.length > 0 && posMs > this.track.length + 2000)
            return;
        this._posBase = posMs;
        this._posTime = nowMs();
        this.emit('position');
    }

    playPause() {
        this._proxy?.PlayPauseRemote(() => {});
    }

    next() {
        this._proxy?.NextRemote(() => {});
    }

    previous() {
        this._proxy?.PreviousRemote(() => {});
    }

    destroy() {
        this._cancellable.cancel();
        if (this._propsId)
            this._proxy.disconnect(this._propsId);
        if (this._seekedId)
            this._proxy.disconnectSignal(this._seekedId);
        this._proxy = null;
    }
}

/**
 * Tracks all MPRIS players on the session bus and picks the one to display.
 * Emits 'update' whenever the active player or its state changes.
 */
export class MprisWatcher extends Signals.EventEmitter {
    constructor() {
        super();
        this.active = null;
        this._players = new Map();
        this._cancellable = new Gio.Cancellable();
    }

    start() {
        this._ownerId = Gio.DBus.session.signal_subscribe('org.freedesktop.DBus',
            'org.freedesktop.DBus', 'NameOwnerChanged', '/org/freedesktop/DBus',
            MPRIS_PREFIX.slice(0, -1), Gio.DBusSignalFlags.MATCH_ARG0_NAMESPACE,
            (_c, _s, _p, _i, _sig, params) => {
                const [name, , newOwner] = params.deepUnpack();
                if (newOwner)
                    this._add(name);
                else
                    this._remove(name);
            });

        Gio.DBus.session.call('org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', 'ListNames', null, new GLib.VariantType('(as)'),
            Gio.DBusCallFlags.NONE, -1, this._cancellable, (conn, res) => {
                try {
                    const [names] = conn.call_finish(res).deepUnpack();
                    names.filter(n => n.startsWith(MPRIS_PREFIX)).forEach(n => this._add(n));
                } catch (e) {
                    if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
                        console.warn(`[lyrics-island] ListNames: ${e.message}`);
                }
            });
    }

    _add(name) {
        if (this._players.has(name))
            return;
        const player = new Player(name);
        player.connect('changed', () => this._pick());
        player.connect('position', () => {
            if (player === this.active)
                this.emit('position');
        });
        this._players.set(name, player);
    }

    _remove(name) {
        const player = this._players.get(name);
        if (!player)
            return;
        player.destroy();
        this._players.delete(name);
        this._pick();
    }

    _pick() {
        let best = null;
        let bestScore = -1;
        for (const p of this._players.values()) {
            if (!p.ready || p.status === 'Stopped' || !p.track.title)
                continue;
            let score = 0;
            if (p.status === 'Playing')
                score += 2;
            if (p.preferred)
                score += 1;
            if (p === this.active)
                score += 0.5;
            if (score > bestScore) {
                best = p;
                bestScore = score;
            }
        }
        this.active = best;
        this.emit('update');
    }

    destroy() {
        this._cancellable.cancel();
        if (this._ownerId)
            Gio.DBus.session.signal_unsubscribe(this._ownerId);
        for (const p of this._players.values())
            p.destroy();
        this._players.clear();
        this.active = null;
    }
}
