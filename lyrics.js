import Gio from 'gi://Gio';
import GdkPixbuf from 'gi://GdkPixbuf';
import GLib from 'gi://GLib';
import Soup from 'gi://Soup?version=3.0';

const BETTER_API = 'https://api.betterlyrics.org';
const LRC_RED = 'https://lrc.red';
const BETTER_RETRY_S = 8;
const NEGATIVE_TTL_MS = 3600 * 1000;

const md5 = s => GLib.compute_checksum_for_string(GLib.ChecksumType.MD5, s, -1);
const enc = encodeURIComponent;

const NOISE = /\s*[([][^)\]]*\b(official(\s+music)?\s+video|official\s+audio|lyrics?|audio|video|mv|m\/v|visuali[sz]er|hd|4k|remaster(ed)?|live(\s+version)?)\b[^)\]]*[)\]]/gi;

function cleanTitle(title) {
    let t = title.replace(NOISE, '').replace(/\s*\|.*$/, '').trim();
    // If title has "Artist - Song", extract Song
    if (t.includes(' - ')) {
        const parts = t.split(/\s+-\s+/);
        if (parts.length === 2 && parts[1].length > 0)
            t = parts[1].trim();
    }
    return t;
}

function cleanArtist(artist) {
    return artist.replace(/\s*-\s*Topic$/i, '').trim();
}

function titleMatches(candidate, target) {
    if (!candidate || !target)
        return false;
    const norm = s => s.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
    const c = norm(candidate);
    const t = norm(target);
    if (!c || !t)
        return false;
    if (c === t || c.includes(t) || t.includes(c))
        return true;
    const wordsC = new Set(c.split(/\s+/));
    const wordsT = new Set(t.split(/\s+/));
    let matchCount = 0;
    for (const w of wordsT) {
        if (wordsC.has(w))
            matchCount++;
    }
    return matchCount >= Math.min(wordsC.size, wordsT.size) * 0.5;
}

/** Parses LRC text into [{t: ms, text}] sorted by time, or null. */
export function parseLrc(lrc) {
    if (!lrc)
        return null;
    const out = [];
    const stamp = /\[(\d+):(\d+(?:[.:]\d+)?)\]/g;
    for (const raw of lrc.split(/\r?\n/)) {
        const times = [];
        let end = 0;
        let m;
        stamp.lastIndex = 0;
        while ((m = stamp.exec(raw))) {
            times.push(parseInt(m[1]) * 60000 + parseFloat(m[2].replace(':', '.')) * 1000);
            end = stamp.lastIndex;
        }
        // Strip enhanced-LRC word timings like <00:41.17>; note-only (instrumental) lines count as empty.
        const text = raw.slice(end).replace(/<\d+:\d+(?:[.:]\d+)?>/g, '').replace(/\s+/g, ' ').trim()
            .replace(/^[♪♫♬♩\s]+$/, '');
        for (const t of times)
            out.push({t, text});
    }
    out.sort((a, b) => a.t - b.t);
    return out.length ? out : null;
}

/** Converts Better Lyrics TTML to line-level LRC text, or null. */
export function ttmlToLrc(ttml) {
    const entities = {amp: '&', lt: '<', gt: '>', quot: '"', apos: "'"};
    const out = [];
    for (const [, begin, body] of (ttml ?? '').matchAll(/<p\b[^>]*?\bbegin="([^"]+)"[^>]*>([\s\S]*?)<\/p>/g)) {
        // begin is "s.mmm", "m:s.mmm" or "h:m:s.mmm"
        const s = begin.split(':').reduce((acc, x) => acc * 60 + parseFloat(x), 0);
        const text = body
            .replace(/<span[^>]*x-(translation|roman)[^>]*>[^<]*<\/span>/g, '')
            .replace(/<[^>]+>/g, '')
            .replace(/&(amp|lt|gt|quot|apos);/g, (_, e) => entities[e])
            .replace(/\s+/g, ' ').trim();
        if (text && s >= 0)
            out.push(`[${Math.floor(s / 60)}:${(s % 60).toFixed(3)}]${text}`);
    }
    return out.length ? out.join('\n') : null;
}

/** Returns the two most different colors [[r, g, b], [r, g, b]] of the art at a file:// URI, or null. */
export function artColors(uri) {
    try {
        // shortcut: synchronous decode on the main loop, fine for small cached covers; go async if track changes hitch.
        const pb = GdkPixbuf.Pixbuf.new_from_file_at_scale(GLib.filename_from_uri(uri)[0], 2, 2, false);
        const px = pb.get_pixels();
        const quad = [0, 1, 2, 3].map(i => {
            const o = (i >> 1) * pb.get_rowstride() + (i & 1) * pb.get_n_channels();
            return [px[o], px[o + 1], px[o + 2]];
        });
        const dist = (a, b) => a.reduce((s, v, i) => s + (v - b[i]) ** 2, 0);
        let best = [quad[0], quad[1]];
        for (const a of quad) {
            for (const b of quad) {
                if (dist(a, b) > dist(...best))
                    best = [a, b];
            }
        }
        return best;
    } catch {
        return null;
    }
}

export class LyricsProvider {
    constructor() {
        this._session = new Soup.Session({
            user_agent: 'lyrics-island/1 (GNOME Shell extension)',
            timeout: 15,
        });
        this._dir = GLib.build_filenamev([GLib.get_user_cache_dir(), 'lyrics-island']);
        this._artDir = GLib.build_filenamev([this._dir, 'art']);
        GLib.mkdir_with_parents(this._artDir, 0o755);
    }

    _request(url, cancellable) {
        return new Promise((resolve, reject) => {
            const msg = Soup.Message.new('GET', url);
            this._session.send_and_read_async(msg, GLib.PRIORITY_DEFAULT, cancellable, (s, res) => {
                try {
                    const bytes = s.send_and_read_finish(res);
                    resolve({status: msg.get_status(), bytes});
                } catch (e) {
                    reject(e);
                }
            });
        });
    }

    async _requestJson(url, cancellable) {
        const {status, bytes} = await this._request(url, cancellable);
        if (status !== 200)
            return null;
        return JSON.parse(new TextDecoder().decode(bytes.get_data() ?? new Uint8Array()));
    }

    _readFile(path, cancellable) {
        return new Promise(resolve => {
            Gio.File.new_for_path(path).load_contents_async(cancellable, (f, res) => {
                try {
                    const [, contents] = f.load_contents_finish(res);
                    resolve(new TextDecoder().decode(contents));
                } catch {
                    resolve(null);
                }
            });
        });
    }

    _writeFile(path, bytes, cancellable) {
        return new Promise((resolve, reject) => {
            Gio.File.new_for_path(path).replace_contents_bytes_async(bytes, null, false,
                Gio.FileCreateFlags.REPLACE_DESTINATION, cancellable, (f, res) => {
                    try {
                        f.replace_contents_finish(res);
                        resolve();
                    } catch (e) {
                        reject(e);
                    }
                });
        });
    }

    /**
     * Returns {synced: [{t, text}] | null, source: string | null}.
     * Network errors reject; "not found" resolves with nulls and is cached.
     */
    async fetch(track, cancellable) {
        const title = cleanTitle(track.title);
        const artist = cleanArtist(track.artist);
        const dur = Math.round(track.length / 1000);
        // Album is part of the key: Better Lyrics matches on it, so a late-arriving album is a different query.
        const cachePath = GLib.build_filenamev([this._dir,
            `${md5(`${artist}\n${title}\n${dur}\n${track.album ?? ''}`)}.json`]);

        const cachedText = await this._readFile(cachePath, cancellable);
        if (cachedText && cachedText.trim().length > 0) {
            try {
                const c = JSON.parse(cachedText);
                // Refetch entries from before sources were recorded and ones from the removed LRCLIB source.
                const usable = 'source' in c && c.source !== 'LRCLIB';
                if (usable && (c.syncedLyrics || Date.now() - c.fetchedAt < NEGATIVE_TTL_MS))
                    return {synced: parseLrc(c.syncedLyrics), source: c.source ?? null};
            } catch {
                // Corrupt cache entry; refetch.
            }
        }

        const firstArtist = artist.split(/,|&|\s+feat\.?\s+|\s+ft\.?\s+/i)[0].trim();
        // A source being down must not block the next one; only cancellation propagates.
        const soft = p => p.catch(e => {
            if (cancellable?.is_cancelled())
                throw e;
            return null;
        });
        const fromTtml = (ttml, source) => {
            const lrc = ttmlToLrc(ttml);
            return lrc ? {syncedLyrics: lrc, source} : null;
        };

        const lrcRed = async () => {
            const res = await soft(this._requestJson(
                `${LRC_RED}/api/v1?track=${enc(title)}&artist=${enc(firstArtist)}`, cancellable));
            const diff = x => Math.abs((x.duration ?? 0) - dur);
            const hit = (res?.results ?? [])
                .filter(x => titleMatches(x.track_name, title) && diff(x) <= 3 && x.lyricsUrl?.startsWith(`${LRC_RED}/`))
                .sort((a, b) => diff(a) - diff(b))[0];
            const r = hit && await soft(this._request(hit.lyricsUrl, cancellable));
            return r?.status === 200
                ? fromTtml(new TextDecoder().decode(r.bytes.get_data() ?? new Uint8Array()), 'lrc.red') : null;
        };
        // Better Lyrics serves cached songs keyless; a miss is 401 -> null.
        const better = async () => fromTtml((await soft(this._requestJson(
            `${BETTER_API}/getLyrics?s=${enc(title)}&a=${enc(artist)}&al=${enc(track.album ?? '')}&d=${dur}`,
            cancellable)))?.ttml, 'Better Lyrics');

        let rec = dur > 0 ? await lrcRed() ?? await better() : null;

        // The Better Lyrics browser extension fills the API cache when the song starts; give it time, then retry once.
        if (!rec && dur > 0) {
            await new Promise(r => GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, BETTER_RETRY_S, () => (r(), GLib.SOURCE_REMOVE)));
            cancellable?.set_error_if_cancelled();
            rec = await better();
        }

        const entry = {
            syncedLyrics: rec?.syncedLyrics ?? null,
            source: rec?.source ?? null,
            fetchedAt: Date.now(),
        };
        this._writeFile(cachePath, new GLib.Bytes(new TextEncoder().encode(JSON.stringify(entry))), null)
            .catch(e => console.warn(`[lyrics-island] cache write: ${e.message}`));
        return {synced: parseLrc(entry.syncedLyrics), source: entry.source};
    }

    /** Downloads album art to the cache and returns a file:// URI, or null. */
    async fetchArt(url, cancellable) {
        if (!url)
            return null;
        if (url.startsWith('file://'))
            return url;
        const file = Gio.File.new_for_path(GLib.build_filenamev([this._artDir, md5(url)]));
        if (file.query_exists(null))
            return file.get_uri();
        const {status, bytes} = await this._request(url, cancellable);
        if (status !== 200)
            return null;
        await this._writeFile(file.get_path(), bytes, cancellable);
        return file.get_uri();
    }

    destroy() {
        this._session.abort();
        this._session = null;
    }
}
