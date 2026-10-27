// QA sessions: a new Claude session presents someone's work to you live, by voice — a beat at
// a time (a few spoken sentences and one slide), and you interrupt it with spoken questions it
// answers from the real code. The QA screen drives it with the ordinary send / messages / keys
// routes and reads its replies aloud; this module only keeps the record of each QA session (one
// folder under the QA dir, config qa.dir: ~/.local/state/bearclaw/qa/<id>/qa.json) and writes the
// presenter's first prompt, which says how a beat looks.
import { randomBytes } from 'node:crypto';
import { promises as fs } from 'node:fs';
import path from 'node:path';

import { HttpError } from './http.mjs';

export const QA_ID_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
/** What every message the QA screen sends starts with; the first prompt carries `c-0`. */
export const QA_MARK = '[qa · ';
const LANGS = { hr: 'Croatian (Latin script)', en: 'English' };

const slug = (s) =>
  String(s ?? '')
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 32)
    .replace(/-+$/, '');

/** A create request → the record's fields, or an error message. */
export function validateQa(body) {
  const topic = typeof body?.topic === 'string' ? body.topic.trim().slice(0, 1500) : '';
  const s = body?.source;
  const source =
    s && typeof s.session === 'string' && s.session
      ? { host: String(s.host ?? ''), session: s.session, title: String(s.title ?? '').slice(0, 200), cwd: typeof s.cwd === 'string' ? s.cwd : null }
      : null;
  if (!topic && !source) return { ok: false, error: 'a QA session needs a topic or a source session' };
  const lang = LANGS[body?.lang] ? body.lang : 'hr';
  const title = (typeof body?.title === 'string' && body.title.trim()) || source?.title || topic.split('\n')[0];
  return { ok: true, title: title.slice(0, 120), topic, source, lang };
}

/** The presenter's first prompt: who it presents to, what, and the shape of every reply. */
export function presenterPrompt(qa) {
  const src = qa.source;
  const id = src?.session?.slice(0, 8);
  const lines = [
    `${QA_MARK}c-0] # QA session: you present, live, by voice`,
    '',
    'You present to the user live, by voice, in a QA session in the bearclaw web app. They see one slide at a time, hear every reply of yours read aloud, and can interrupt you at any moment with a spoken question. Make it feel like a colleague presenting their work at a whiteboard: short spoken beats, a slide that changes as you talk, the real code on screen when it helps.',
    '',
    qa.topic ? `Topic: ${qa.topic}` : `Topic: the work of the session below — what it built or decided, why, and how it works.`,
  ];
  if (src) {
    lines.push(
      `Source: the Claude session "${src.title || id}" (host ${src.host || 'this one'}, id ${src.session}${src.cwd ? `, in ${src.cwd}` : ''}) did this work. Learn it before you start: \`bearclaw brief ${id}\` (its summary, links and todos), \`bearclaw peek ${id} --lines 150\`, any artifact or document it links (read it), and the code it touched. Never message, interrupt or change that session.`,
    );
  }
  lines.push(
    '',
    '## Every reply is one beat',
    '',
    `- First the spoken part: two to five short sentences, said aloud. No markdown, lists, headings, code, paths or ids in it. Speak ${LANGS[qa.lang] ?? LANGS.hr}; keep English technical words as they are.`,
    '- Then exactly one slide, as a fenced block at the very end of the reply:',
    '',
    '```slide',
    '{"title": "...", "points": ["..."], "code": {"path": "src/tools/x.ts", "start": 120, "text": "...", "highlight": [3, 4]}, "diagram": {"nodes": [{"id": "main", "label": "Main agent", "note": "one model"}, {"id": "sub", "label": "Subagent"}], "edges": [["main", "sub", "delegate_task"]]}}',
    '```',
    '',
    '  `title` always, then any of: `points` (up to 5, short), `code` (up to 18 lines copied from the real file; `start` is the line number of its first line; `highlight` lists lines within the snippet, from 1), `diagram` (up to 8 nodes; edges are [from, to, label?]). One idea per slide.',
    '- The app sends "[qa · c-N] continue" for the next beat of the talk. Keep the talk to 6–10 beats. On the last one, invite questions and put `"end": true` in its slide.',
    '- "[qa · q-…] question …" means the user interrupted you. Answer that question now, in this beat. If it needs the code, read the actual code first (files, git, gh), show what you found on the slide (the real snippet, or a diagram of it), then speak the answer. On the next continue, pick the talk back up where it fits, reshaping the rest of the plan around what they asked.',
    '- You only read: change no files and run nothing that writes, so no permission prompt stops the talk.',
    '',
    'Start now: learn the source, plan the talk, and reply with beat 1: a title slide with what the talk will cover.',
  );
  return lines.join('\n');
}

/**
 * @param {{ dir: string, now?: () => number }} opts
 * @returns {{ dir, list(), get(id), create(body) }}
 */
export function createQa({ dir, now = Date.now }) {
  function folder(id) {
    if (!QA_ID_RE.test(String(id))) throw new HttpError(`unknown QA session: ${id}`, 404);
    return path.join(dir, id);
  }

  async function read(id) {
    try {
      return JSON.parse(await fs.readFile(path.join(folder(id), 'qa.json'), 'utf8'));
    } catch (err) {
      if (err.code === 'ENOENT' || err.code === 'ENOTDIR' || err instanceof SyntaxError) throw new HttpError(`unknown QA session: ${id}`, 404);
      throw err;
    }
  }

  return {
    dir,

    /** Every QA session, newest first. */
    async list() {
      let ids;
      try {
        ids = (await fs.readdir(dir)).filter((n) => QA_ID_RE.test(n));
      } catch (err) {
        if (err.code === 'ENOENT') return [];
        throw err;
      }
      const out = [];
      for (const id of ids) {
        try {
          out.push(await read(id));
        } catch {
          /* not a QA session */
        }
      }
      return out.sort((a, b) => (b.createdAt ?? 0) - (a.createdAt ?? 0));
    },

    get: read,

    /** A new QA session's record and the presenter's first prompt (the caller spawns it). */
    async create(body) {
      const check = validateQa(body);
      if (!check.ok) throw new HttpError(check.error, 400);
      const { ok: _ok, ...fields } = check;
      const id = `${slug(fields.title) || 'qa'}-${randomBytes(2).toString('hex')}`;
      const qa = { id, ...fields, presenter: { tmux: `qa-${id}`.slice(0, 40) }, createdAt: now() };
      await fs.mkdir(folder(id), { recursive: true });
      await fs.writeFile(path.join(folder(id), 'qa.json'), `${JSON.stringify(qa, null, 1)}\n`);
      return { ...qa, prompt: presenterPrompt(qa) };
    },
  };
}
