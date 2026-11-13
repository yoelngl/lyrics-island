import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import {artColors} from './lyrics.js';

const EXPANDED_WIDTH = 400;
const COLLAPSE_DELAY_MS = 1500;
const GRADIENT_PERIOD_S = 8;
const EASE = Clutter.AnimationMode.EASE_OUT_QUAD;

function formatTime(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

const Bars = GObject.registerClass(
class Bars extends St.BoxLayout {
    _init(maxHeight) {
        super._init({style_class: 'li-bars', y_align: Clutter.ActorAlign.CENTER, height: maxHeight});
        this._maxHeight = maxHeight;
        this._bars = [];
        for (let i = 0; i < 4; i++) {
            const bar = new St.Widget({style_class: 'li-bar', y_align: Clutter.ActorAlign.CENTER, height: 3});
            this._bars.push(bar);
            this.add_child(bar);
        }
        this._timer = 0;
        this.connect('destroy', () => this._stopTimer());
    }

    _stopTimer() {
        if (this._timer)
            GLib.source_remove(this._timer);
        this._timer = 0;
    }

    setPlaying(playing) {
        if (playing && !this._timer) {
            this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 180, () => {
                for (const bar of this._bars) {
                    bar.ease({
                        height: Math.round(3 + Math.random() * (this._maxHeight - 3)),
                        duration: 170,
                        mode: Clutter.AnimationMode.EASE_IN_OUT_QUAD,
                    });
                }
                return GLib.SOURCE_CONTINUE;
            });
        } else if (!playing && this._timer) {
            this._stopTimer();
            for (const bar of this._bars)
                bar.ease({height: 3, duration: 200, mode: EASE});
        }
    }
});

export const Island = GObject.registerClass({
    Signals: {'play-pause': {}, 'next': {}, 'previous': {}},
}, class Island extends St.Widget {
    _init(settings) {
        super._init({
            style_class: 'lyrics-island',
            reactive: true,
            track_hover: true,
            clip_to_allocation: true,
            layout_manager: new Clutter.BinLayout(),
            visible: false,
            opacity: 0,
        });
        this._settings = settings;
        this._expanded = false;
        this._collapseId = 0;
        this._destroyed = false;
        this._pillHeight = Math.max(22, Main.panel.height - 6);

        this._buildCompact();
        this._buildExpanded();

        this.connect('notify::width', () => this._reposition());
        this.connect('notify::hover', () => this._onHover());
        this.connect('button-press-event', () => {
            if (this._expanded)
                this.collapse();
            else
                this.expand();
            return Clutter.EVENT_STOP;
        });
        this._monitorsId = Main.layoutManager.connect('monitors-changed', () => {
            this._pillHeight = Math.max(22, Main.panel.height - 6);
            this._resizeCompact(false);
            this._reposition();
        });
        this._interface = new Gio.Settings({schema_id: 'org.gnome.desktop.interface'});
        this._schemeId = this._interface.connect('changed::color-scheme', () => this._applySettings());
        this._settingsId = this._settings.connect('changed', () => this._applySettings());
        this.connect('destroy', () => this._onDestroy());

        this._applySettings();
        this.set_size(180, this._pillHeight);
        this._reposition();
    }

    _buildCompact() {
        const h = this._pillHeight;
        this._compact = new St.BoxLayout({
            style_class: 'li-compact',
            x_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.START,
            height: h,
        });
        this._artSmall = new St.Bin({
            style_class: 'li-art-small',
            width: h - 8,
            height: h - 8,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._line = new St.Label({
            style_class: 'li-line',
            // Takes the spare width so the bars stay at the right edge for short lines.
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._line.clutter_text.ellipsize = Pango.EllipsizeMode.NONE;
        this._barsSmall = new Bars(Math.round(h * 0.5));

        this._compact.add_child(this._artSmall);
        this._compact.add_child(this._line);
        this._compact.add_child(this._barsSmall);
        this.add_child(this._compact);
    }

    _buildExpanded() {
        this._expandedBox = new St.BoxLayout({
            style_class: 'li-expanded',
            vertical: true,
            width: EXPANDED_WIDTH,
            x_align: Clutter.ActorAlign.START,
            y_align: Clutter.ActorAlign.START,
            visible: false,
            opacity: 0,
        });

        const header = new St.BoxLayout({style_class: 'li-header'});
        this._artLarge = new St.Bin({style_class: 'li-art-large', width: 56, height: 56});
        const meta = new St.BoxLayout({vertical: true, x_expand: true, y_align: Clutter.ActorAlign.CENTER});
        this._title = new St.Label({style_class: 'li-title'});
        this._artist = new St.Label({style_class: 'li-artist'});
        for (const l of [this._title, this._artist])
            l.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        meta.add_child(this._title);
        meta.add_child(this._artist);
        this._barsLarge = new Bars(20);
        header.add_child(this._artLarge);
        header.add_child(meta);
        header.add_child(this._barsLarge);

        const lyrics = new St.BoxLayout({style_class: 'li-lyrics', vertical: true});
        this._prev = new St.Label({style_class: 'li-prev'});
        this._cur = new St.Label({style_class: 'li-current'});
        this._next = new St.Label({style_class: 'li-next'});
        for (const l of [this._prev, this._next])
            l.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        this._cur.clutter_text.line_wrap = true;
        this._cur.clutter_text.ellipsize = Pango.EllipsizeMode.NONE;
        lyrics.add_child(this._prev);
        lyrics.add_child(this._cur);
        lyrics.add_child(this._next);

        const progress = new St.BoxLayout({style_class: 'li-progress'});
        this._elapsed = new St.Label({style_class: 'li-time', y_align: Clutter.ActorAlign.CENTER});
        this._total = new St.Label({style_class: 'li-time', y_align: Clutter.ActorAlign.CENTER});
        this._track = new St.Widget({style_class: 'li-track', x_expand: true, y_align: Clutter.ActorAlign.CENTER});
        this._fill = new St.Widget({style_class: 'li-fill', width: 0});
        this._track.add_child(this._fill);
        this._frac = 0;
        // The track resizes while the island animates open; recompute the fill instead of waiting for the next tick.
        this._track.connect('notify::width', () => this._updateFill());
        progress.add_child(this._elapsed);
        progress.add_child(this._track);
        progress.add_child(this._total);

        const controls = new St.BoxLayout({style_class: 'li-controls', x_align: Clutter.ActorAlign.CENTER});
        const button = (icon, signal) => {
            const b = new St.Button({
                style_class: 'li-control',
                child: new St.Icon({icon_name: icon, icon_size: 20}),
            });
            b.connect('clicked', () => this.emit(signal));
            controls.add_child(b);
            return b;
        };
        button('media-skip-backward-symbolic', 'previous');
        this._playButton = button('media-playback-start-symbolic', 'play-pause');
        button('media-skip-forward-symbolic', 'next');

        // Always allocated (even when empty) so the expanded height doesn't change when lyrics arrive.
        this._credit = new St.Label({style_class: 'li-credit', x_align: Clutter.ActorAlign.CENTER});

        for (const child of [header, lyrics, progress, controls, this._credit])
            this._expandedBox.add_child(child);
        this.add_child(this._expandedBox);
    }

    _applySettings() {
        this._line.set_style(`font-size: ${this._settings.get_int('font-size')}pt;`);
        const theme = this._settings.get_string('theme');
        // GNOME's "Light" style is color-scheme 'default', so anything but prefer-dark is light.
        const light = theme === 'light' ||
            (theme === 'system' && this._interface.get_string('color-scheme') !== 'prefer-dark');
        if (light)
            this.add_style_class_name('light');
        else
            this.remove_style_class_name('light');
        this._resizeCompact(true);
    }

    _reposition() {
        const mon = Main.layoutManager.primaryMonitor;
        if (!mon)
            return;
        this.set_position(
            mon.x + Math.floor((mon.width - this.width) / 2),
            mon.y + Math.floor((Main.panel.height - this._pillHeight) / 2));
    }

    _resizeCompact(animate) {
        if (this._expanded || !this.get_stage())
            return;
        const mon = Main.layoutManager.primaryMonitor;
        const screenMax = mon ? Math.floor(mon.width * 0.75) : 1000;
        const userMax = this._settings.get_int('max-width');
        const max = Math.min(Math.max(userMax, 850), screenMax);

        const h = this._pillHeight;
        const [, natural] = this._line.get_preferred_width(-1);
        const w = Math.round(Math.min(max, Math.max(160, natural + h + 44)));
        if (Math.abs(w - this.width) < 4 && this.height === h)
            return;
        if (animate)
            this.ease({width: w, height: h, duration: 240, mode: EASE});
        else
            this.set_size(w, h);
    }

    _swapText(label, text, after) {
        if (!label.get_stage() || !this.visible || !label.visible) {
            label.text = text;
            delete label._pendingText;
            label.opacity = 255;
            label.translation_y = 0;
            after?.();
            return;
        }
        if (label._pendingText !== undefined) {
            label._pendingText = text;
            return;
        }
        if (label.text === text)
            return;
        label._pendingText = text;
        label.ease({
            opacity: 0,
            translation_y: -6,
            duration: 110,
            mode: EASE,
            onStopped: () => {
                if (this._destroyed)
                    return;
                label.text = label._pendingText ?? text;
                delete label._pendingText;
                label.translation_y = 6;
                after?.();
                label.ease({opacity: 255, translation_y: 0, duration: 200, mode: EASE});
            },
        });
    }

    _onHover() {
        if (this._collapseId) {
            GLib.source_remove(this._collapseId);
            this._collapseId = 0;
        }
        if (this._expanded && !this.hover) {
            this._collapseId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, COLLAPSE_DELAY_MS, () => {
                this._collapseId = 0;
                this.collapse();
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    expand() {
        if (this._expanded)
            return;
        this._expanded = true;
        this.add_style_class_name('expanded');
        this._expandedBox.show();
        this._cur.opacity = 255;
        this._cur.translation_y = 0;
        delete this._cur._pendingText;
        const [, height] = this._expandedBox.get_preferred_height(EXPANDED_WIDTH);
        this._compact.ease({opacity: 0, duration: 100, mode: EASE});
        this.ease({
            width: EXPANDED_WIDTH,
            height: Math.round(height),
            duration: 420,
            mode: Clutter.AnimationMode.EASE_OUT_BACK,
        });
        this._expandedBox.ease({opacity: 255, delay: 180, duration: 220, mode: EASE});
    }

    collapse() {
        if (!this._expanded)
            return;
        this._expanded = false;
        this._line.opacity = 255;
        this._line.translation_y = 0;
        delete this._line._pendingText;
        this._expandedBox.ease({
            opacity: 0,
            duration: 100,
            mode: EASE,
            onStopped: () => {
                if (this._destroyed || this._expanded)
                    return;
                this._expandedBox.hide();
                this.remove_style_class_name('expanded');
            },
        });
        this._compact.ease({opacity: 255, delay: 150, duration: 200, mode: EASE});
        this._resizeCompact(true);
    }

    /** Runs the art-colored gradient (pill and expanded) while the island is shown and the art has colors. */
    _updateGradient() {
        const on = this.visible && this._artColors;
        if (on && !this._gradientId) {
            this._gradientId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
                this._paintGradient();
                return GLib.SOURCE_CONTINUE;
            });
            this._paintGradient();
        } else if (!on && this._gradientId) {
            GLib.source_remove(this._gradientId);
            this._gradientId = 0;
            this.set_style(null);
        }
    }

    _paintGradient() {
        const [a, b] = this._artColors;
        const light = this.has_style_class_name('light');
        // Keep the theme's text readable: darken for white text, wash out for black.
        const tone = v => Math.round(light ? v + (255 - v) * 0.65 : v * 0.5);
        const mix = k => `rgb(${a.map((v, i) => tone(v + (b[i] - v) * k)).join(',')})`;
        // The two ends trade colors back and forth, which reads as the gradient sweeping across.
        const k = (Math.sin(GLib.get_monotonic_time() / 1e6 * 2 * Math.PI / GRADIENT_PERIOD_S) + 1) / 2;
        this.set_style('background-gradient-direction: horizontal; ' +
            `background-gradient-start: ${mix(k)}; background-gradient-end: ${mix(1 - k)};`);
    }

    showIsland() {
        if (this.visible && this.opacity === 255)
            return;
        this.show();
        this._updateGradient();
        this.ease({opacity: 255, duration: 250, mode: EASE});
    }

    hideIsland() {
        if (!this.visible)
            return;
        this.collapse();
        this.ease({
            opacity: 0,
            duration: 250,
            mode: EASE,
            onStopped: finished => {
                if (finished && !this._destroyed) {
                    this.hide();
                    this._updateGradient();
                }
            },
        });
    }

    setTrack({title, artist}) {
        this._title.text = title;
        this._artist.text = artist;
    }

    /** Shows which service the lyrics came from, or nothing for null. */
    setCredit(source) {
        this._credit.text = source ? `Lyrics · ${source}` : '';
    }

    setArt(uri) {
        const style = uri ? `background-image: url("${uri}");` : '';
        this._artSmall.set_style(style);
        this._artLarge.set_style(style);
        this._artColors = uri ? artColors(uri) : null;
        this._updateGradient();
    }

    setPlaying(playing) {
        this._barsSmall.setPlaying(playing);
        this._barsLarge.setPlaying(playing);
        this._playButton.child.icon_name = playing
            ? 'media-playback-pause-symbolic' : 'media-playback-start-symbolic';
    }

    setLines(compactText, prev, cur, next) {
        this._swapText(this._line, compactText, () => this._resizeCompact(true));
        this._prev.text = prev;
        if (this._expanded)
            this._swapText(this._cur, cur);
        else
            this._cur.text = cur;
        this._next.text = next;
    }

    setProgress(pos, length) {
        this._elapsed.text = formatTime(pos);
        this._total.text = length > 0 ? formatTime(length) : '--:--';
        this._frac = length > 0 ? Math.min(1, Math.max(0, pos / length)) : 0;
        this._updateFill();
    }

    _updateFill() {
        this._fill.width = Math.round(this._track.width * this._frac);
    }

    _onDestroy() {
        this._destroyed = true;
        if (this._collapseId)
            GLib.source_remove(this._collapseId);
        if (this._gradientId)
            GLib.source_remove(this._gradientId);
        Main.layoutManager.disconnect(this._monitorsId);
        this._settings.disconnect(this._settingsId);
        this._interface.disconnect(this._schemeId);
    }
});
