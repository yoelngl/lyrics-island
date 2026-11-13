import Gio from 'gi://Gio';
import GdkPixbuf from 'gi://GdkPixbuf';
import GLib from 'gi://GLib';
import Soup from 'gi://Soup?version=3.0';

const BETTER_API = 'https://api.betterlyrics.org';
const LRC_RED = 'https://lrc.red';
const KUGOU = 'https://lyrics.kugou.com';
const LRCLIB = 'https://lrclib.net';
const NETEASE = 'https://music.163.com';
const QQ = 'https://c.y.qq.com';
const BETTER_RETRY_S = 8;
const NEGATIVE_TTL_MS = 3600 * 1000;
// Credit lines that sources (KuGou, QQ, Netease) put in the timed lyrics, e.g. "Lyrics by：…", "作曲：…".
const CREDIT = /^[^:：]{0,30}(lyrics|lyricist|written|writer|compos|music|arrang|produc|mix|master|record|engineer|vocal|guitar|bass|drum|keyboard|piano|string|publish|label|词|曲|编|制作|监制|混音|母带|和声|合声|吉他|贝斯|鼓|录音|出品|发行|企划|演唱|原唱)[^:：]{0,30}[:：]/i;

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
        if (CREDIT.test(text))
            continue;
        for (const t of times)
            out.push({t, text});
    }
    out.sort((a, b) => a.t - b.t);
    // Sources often open with a "Title - Artist" header line.
    const first = out.findIndex(l => l.text);
    if (first >= 0 && / - /.test(out[first].text))
        out.splice(first, 1);
    return out.some(l => l.text) ? out : null;
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

    _request(url, cancellable, headers = {}) {
        return new Promise((resolve, reject) => {
            const msg = Soup.Message.new('GET', url);
            for (const [k, v] of Object.entries(headers))
                msg.get_request_headers().append(k, v);
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

    async _requestJson(url, cancellable, headers) {
        const {status, bytes} = await this._request(url, cancellable, headers);
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
                // Refetch entries from before sources were recorded.
                const usable = 'source' in c;
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
        const kugou = async () => {
            const res = await soft(this._requestJson(
                `${KUGOU}/search?ver=1&man=yes&client=pc&keyword=${enc(`${firstArtist} - ${title}`)}&duration=${dur * 1000}`,
                cancellable));
            const diff = x => Math.abs((x.duration ?? 0) / 1000 - dur);
            const hit = (res?.candidates ?? [])
                .filter(x => titleMatches(x.song, title) && diff(x) <= 3)
                .sort((a, b) => diff(a) - diff(b))[0];
            const lrc = hit && (await soft(this._requestJson(
                `${KUGOU}/download?ver=1&client=pc&fmt=lrc&charset=utf8&id=${enc(hit.id)}&accesskey=${enc(hit.accesskey)}`,
                cancellable)))?.content;
            const text = lrc && new TextDecoder().decode(GLib.base64_decode(lrc));
            return parseLrc(text) ? {syncedLyrics: text, source: 'KuGou'} : null;
        };

        // Only entries with timestamps count; plain/unsynced lyrics are dropped. Candidates must match title, artist and length.
        const synced = (text, source) => parseLrc(text) ? {syncedLyrics: text, source} : null;
        const picks = (list, name, singers, secs) => (list ?? [])
            .filter(x => titleMatches(name(x), title) && titleMatches(singers(x), firstArtist) && Math.abs(secs(x) - dur) <= 3)
            .sort((a, b) => Math.abs(secs(a) - dur) - Math.abs(secs(b) - dur));
        const lrclib = async () => {
            const res = await soft(this._requestJson(
                `${LRCLIB}/api/search?track_name=${enc(title)}&artist_name=${enc(firstArtist)}`, cancellable));
            const hit = picks(Array.isArray(res) ? res.filter(x => x.syncedLyrics) : [],
                x => x.trackName, x => x.artistName, x => x.duration)[0];
            return hit ? synced(hit.syncedLyrics, 'LRCLIB') : null;
        };
        const netease = async () => {
            const res = await soft(this._requestJson(
                `${NETEASE}/api/search/get?s=${enc(`${firstArtist} ${title}`)}&type=1&limit=10`, cancellable));
            // Copyright-restricted entries come back with empty lyrics; try the next match.
            for (const hit of picks(res?.result?.songs, x => x.name, x => (x.artists ?? []).map(a => a.name).join(' '), x => x.duration / 1000).slice(0, 3)) {
                const lrc = (await soft(this._requestJson(`${NETEASE}/api/song/lyric?id=${hit.id}&lv=1`, cancellable)))?.lrc?.lyric;
                if (synced(lrc))
                    return synced(lrc, 'Netease Cloud Music');
            }
            return null;
        };
        const qq = async () => {
            const headers = {Referer: 'https://y.qq.com/'};
            const res = await soft(this._requestJson(
                `${QQ}/soso/fcgi-bin/client_search_cp?w=${enc(`${firstArtist} ${title}`)}&format=json&n=10&p=1&cr=1`,
                cancellable, headers));
            const hit = picks(res?.data?.song?.list, x => x.songname, x => (x.singer ?? []).map(s => s.name).join(' '), x => x.interval)[0];
            const b64 = hit && (await soft(this._requestJson(
                `${QQ}/lyric/fcgi-bin/fcg_query_lyric_new.fcg?songmid=${hit.songmid}&format=json&nobase64=0`,
                cancellable, headers)))?.lyric;
            return synced(b64 && new TextDecoder().decode(GLib.base64_decode(b64)), 'QQ Music');
        };

        let rec = dur > 0
            ? await lrcRed() ?? await better() ?? await lrclib() ?? await kugou() ?? await netease() ?? await qq() : null;

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
