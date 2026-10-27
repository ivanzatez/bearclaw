// Videos a session made about its work: narrated scenes you watch on the Videos screen, pause,
// and ask about by voice — the question goes to the session that made the video, its spoken
// answer comes back, and it can put an answer slide on the film. One folder per video under the
// videos dir (config videos.dir, ~/.local/state/bearclaw/videos):
//   video.json   { title, session, host?, audio, scenes: [{ id, name, start, dur, visual?, anchors?,
//                  lines: [{ t0, t1, text }] }] }   (seconds on the narration's clock; see video-player.mjs)
//   scenes.js    the scenes' code (lib/video-player.mjs runs it in a sandboxed page)
//   <audio>      the narration the scenes follow (m4a, mp3 or wav), named by video.json's `audio`
//   slides/      answer slides the session writes while you ask: <name>.json { title, points?, code? }
import { promises as fs } from 'node:fs';
import path from 'node:path';

import { HttpError } from './http.mjs';
import { playerHtml } from './video-player.mjs';

export const VIDEO_ID_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
const AUDIO_TYPES = { '.m4a': 'audio/mp4', '.mp3': 'audio/mpeg', '.wav': 'audio/wav' };
const SLIDE_RE = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}\.json$/;
const MAX_POINTS = 8;
const MAX_CODE = 4000;

const num = (v, d = 0) => (Number.isFinite(Number(v)) ? Number(v) : d);

/** A scene as the player and the UI read it: numbers numbers, lines a list. */
function scene(s, i) {
  return {
    id: String(s?.id ?? `scene-${i + 1}`),
    name: String(s?.name ?? s?.id ?? `Scene ${i + 1}`),
    start: num(s?.start),
    dur: num(s?.dur),
    visual: num(s?.visual, num(s?.dur)),
    anchors: Array.isArray(s?.anchors) ? s.anchors.map((a) => (a == null ? null : num(a))) : null,
    lines: Array.isArray(s?.lines) ? s.lines.map((l) => ({ t0: num(l?.t0), t1: num(l?.t1), text: String(l?.text ?? '') })) : [],
  };
}

/** An answer slide as the player shows it; null for one that is not a slide (yet). */
export function parseSlide(text) {
  let s;
  try {
    s = JSON.parse(text);
  } catch {
    return null;
  }
  if (!s || typeof s !== 'object' || typeof s.title !== 'string' || !s.title.trim()) return null;
  return {
    title: s.title.trim().slice(0, 200),
    points: Array.isArray(s.points) ? s.points.map((p) => String(p).slice(0, 300)).slice(0, MAX_POINTS) : [],
    code: typeof s.code === 'string' && s.code.trim() ? s.code.slice(0, MAX_CODE) : null,
  };
}

/**
 * @param {{ dir: string }} opts  the videos dir
 * @returns {{ dir, list(), get(id), audio(id), player(id) }}
 */
export function createVideos({ dir }) {
  function folder(id) {
    if (!VIDEO_ID_RE.test(String(id))) throw new HttpError(`unknown video: ${id}`, 404);
    return path.join(dir, id);
  }

  async function readVideo(id) {
    let raw;
    try {
      raw = await fs.readFile(path.join(folder(id), 'video.json'), 'utf8');
    } catch (err) {
      if (err.code === 'ENOENT' || err.code === 'ENOTDIR') throw new HttpError(`unknown video: ${id}`, 404);
      throw err;
    }
    let v;
    try {
      v = JSON.parse(raw);
    } catch (err) {
      throw new HttpError(`video ${id}: video.json is not valid JSON (${err.message})`, 500);
    }
    const scenes = (Array.isArray(v.scenes) ? v.scenes : []).map(scene);
    return {
      id,
      title: String(v.title || id),
      session: typeof v.session === 'string' && v.session ? v.session : null,
      host: typeof v.host === 'string' && v.host ? v.host : null,
      audio: typeof v.audio === 'string' && v.audio ? path.basename(v.audio) : null,
      duration: scenes.reduce((m, s) => Math.max(m, s.start + s.dur), 0),
      scenes,
    };
  }

  async function slides(id) {
    const d = path.join(folder(id), 'slides');
    let names;
    try {
      names = (await fs.readdir(d)).filter((n) => SLIDE_RE.test(n));
    } catch (err) {
      if (err.code === 'ENOENT') return [];
      throw err;
    }
    const out = [];
    for (const name of names) {
      try {
        const file = path.join(d, name);
        const [stat, text] = await Promise.all([fs.stat(file), fs.readFile(file, 'utf8')]);
        const s = parseSlide(text);
        // Half-written, or not a slide: the next poll looks again.
        if (s) out.push({ name, mtime: stat.mtimeMs, ...s });
      } catch {
        /* gone between readdir and read */
      }
    }
    return out.sort((a, b) => a.mtime - b.mtime);
  }

  return {
    dir,

    /** Every video, newest first: { id, title, session, host, duration, scenes, updatedAt }. */
    async list() {
      let ids;
      try {
        ids = (await fs.readdir(dir)).filter((n) => VIDEO_ID_RE.test(n));
      } catch (err) {
        if (err.code === 'ENOENT') return [];
        throw err;
      }
      const out = [];
      for (const id of ids) {
        try {
          const [v, stat] = await Promise.all([readVideo(id), fs.stat(path.join(dir, id, 'video.json'))]);
          out.push({ id, title: v.title, session: v.session, host: v.host, duration: v.duration, scenes: v.scenes.length, updatedAt: stat.mtimeMs });
        } catch {
          /* not a video (no or broken video.json) */
        }
      }
      return out.sort((a, b) => b.updatedAt - a.updatedAt);
    },

    /** One video with its slides and its folder (where the session writes slides/). */
    async get(id) {
      const v = await readVideo(id);
      return { ...v, dir: folder(id), slidesDir: path.join(folder(id), 'slides'), slides: await slides(id) };
    },

    /** The narration: { type, bytes }. */
    async audio(id) {
      const v = await readVideo(id);
      const type = v.audio ? AUDIO_TYPES[path.extname(v.audio).toLowerCase()] : null;
      if (!type) throw new HttpError(`video ${id} has no narration`, 404);
      try {
        return { type, bytes: await fs.readFile(path.join(folder(id), v.audio)) };
      } catch (err) {
        if (err.code === 'ENOENT') throw new HttpError(`video ${id}: ${v.audio} is missing`, 404);
        throw err;
      }
    },

    /** The player page (HTML) with the video's scenes. */
    async player(id) {
      const v = await readVideo(id);
      const code = await fs.readFile(path.join(folder(id), 'scenes.js'), 'utf8').catch((err) => {
        if (err.code === 'ENOENT') return '';
        throw err;
      });
      return playerHtml({ video: v, scenesCode: code });
    },
  };
}
