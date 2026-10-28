// A QA session, live: the presenter talks to the user through its MCP tools (bin/qa-mcp.mjs →
// /api/qa/<id>/mcp/*) and the QA screen follows an ordered stream of what to show and say
// (/api/qa/<id>/events). One room per QA session, kept in memory and on disk (the QA folder's
// events.jsonl and state.json: what was said, and how far the user has heard it), so that the
// screen picks up where you left off, after you leave the page or the server restarts:
//   say(text)    queued, its voice made at once; returns when the one before it has been heard
//                (so the presenter prepares the next sentence while this one plays)
//   show(slide)  queued in the same order as the sentences
//   listen()     the presenter has nothing more to say: waits for a question, or "continue"
//   status(text) what it is doing right now (shown at once, not queued)
// The user interrupts: what is queued is dropped and every presenter call holds until the
// question arrives, which the next (or the waiting) call returns instead of its usual answer,
// with an id. The presenter decides several calls at once (show, say, say), so the rest of the
// batch it decided before it saw the question would still be said: until it calls listen(), a
// say or show that does not pass `answering: <that id>` is dropped.

import { appendFile, writeFile } from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import path from 'node:path';

/** Longest a presenter call waits before returning (well under any MCP tool timeout). */
export const HOLD_MS = 45 * 1000;
/** Longest an events poll waits. */
export const EVENTS_MS = 20 * 1000;
const KEEP_EVENTS = 3000;
const ID_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;

/**
 * @param {{ speaker: { speak(text): Promise<Buffer> } | null, dir?: string | null, log?: (s: string) => void, holdMs?: number }} deps
 *   dir   the QA dir (lib/qa.mjs): each room is saved in its QA session's folder; null keeps it in memory only
 */
export function createQaLive({ speaker, dir = null, log = () => {}, holdMs = HOLD_MS, eventsMs = EVENTS_MS } = {}) {
  const rooms = new Map();
  const file = (id, name) => (dir && ID_RE.test(id) ? path.join(dir, id, name) : null);

  /** What a room saved: its events and how far the user has heard. */
  function load(id) {
    const events = [];
    let heard = 0;
    try {
      for (const line of readFileSync(file(id, 'events.jsonl') ?? '', 'utf8').split('\n')) {
        try {
          if (line.trim()) events.push(JSON.parse(line));
        } catch {
          /* a line cut short by a crash */
        }
      }
    } catch {
      /* new, or nothing saved */
    }
    try {
      heard = Number(JSON.parse(readFileSync(file(id, 'state.json') ?? '', 'utf8')).heard) || 0;
    } catch {
      /* new */
    }
    return { events, heard };
  }

  function room(id) {
    let r = rooms.get(id);
    if (!r) {
      const saved = load(id);
      const says = new Map();
      const order = [];
      for (const e of saved.events) {
        if (e.type !== 'say') continue;
        says.set(String(e.seq), { text: e.text, audio: null, state: e.seq <= saved.heard ? 'played' : 'queued' });
        order.push(String(e.seq));
      }
      r = {
        id,
        seq: saved.events.at(-1)?.seq ?? 0,
        events: saved.events.slice(-KEEP_EVENTS),
        heard: saved.heard,
        says,
        order,
        hold: false,
        question: null,
        cont: false,
        wakers: new Set(),
        qn: 0,
        answering: null,
        asked: null,
        away: false,
        writes: Promise.resolve(),
      };
      rooms.set(id, r);
    }
    return r;
  }

  /** Save in order, one write after another; a failed write is logged, never thrown. */
  function save(r, name, write) {
    const f = file(r.id, name);
    if (!f) return;
    r.writes = r.writes.then(() => write(f)).catch((err) => log(`[bearclaw-web] qa ${r.id}: ${name}: ${err?.message ?? err}`));
  }

  const wake = (r) => {
    for (const w of [...r.wakers]) w();
  };

  function push(r, ev) {
    r.seq += 1;
    const e = { seq: r.seq, at: Date.now(), ...ev };
    r.events.push(e);
    if (r.events.length > KEEP_EVENTS) r.events.splice(0, r.events.length - KEEP_EVENTS);
    save(r, 'events.jsonl', (f) => appendFile(f, `${JSON.stringify(e)}\n`));
    wake(r);
    return r.seq;
  }

  /** Resolve when `ready()` returns a value (re-checked on every change), or `fallback` after `ms`. */
  function until(r, ready, ms, fallback) {
    const now = ready();
    if (now !== undefined) return Promise.resolve(now);
    return new Promise((resolve) => {
      const done = (v) => {
        clearTimeout(timer);
        r.wakers.delete(check);
        resolve(v);
      };
      const check = () => {
        const v = ready();
        if (v !== undefined) done(v);
      };
      const timer = setTimeout(() => done(fallback), ms);
      timer.unref?.();
      r.wakers.add(check);
    });
  }

  /** The user's question, taken: what a held or interrupted presenter call returns. */
  const take = (r) => {
    if (r.question === null) return undefined;
    const q = r.question;
    r.question = null;
    r.qn += 1;
    r.answering = `q${r.qn}`;
    r.asked = q;
    return { interrupted: true, question: q, id: r.answering };
  };

  /** A say or show decided before the presenter saw the question it is answering: not said. */
  const stale = (r, answering) => (r.answering && answering !== r.answering ? { dropped: true, question: r.asked, id: r.answering } : undefined);

  return {
    /** The events after `after`, waiting up to `eventsMs` for one. */
    async events(id, after = 0) {
      const r = room(id);
      const fresh = () => {
        const list = r.events.filter((e) => e.seq > after);
        return list.length ? { seq: r.seq, heard: r.heard, events: list } : undefined;
      };
      // A client from before a server restart (seq ahead of ours) starts over.
      if (after > r.seq) after = 0;
      return until(r, fresh, eventsMs, { seq: r.seq, heard: r.heard, events: [] });
    },

    /** A sentence's voice (mp3), made when it was said. */
    async audio(id, sid) {
      const s = room(id).says.get(String(sid));
      if (!s) return null;
      // Said before a restart: its voice is made again when it is asked for.
      if (!s.audio) {
        s.audio = speaker ? speaker.speak(s.text) : Promise.reject(new Error('no voice on this server'));
        s.audio.catch(() => {});
      }
      return s.audio;
    },

    // --- the presenter (MCP) --------------------------------------------------------------

    async say(id, text, answering) {
      const r = room(id);
      const now = take(r) ?? stale(r, answering);
      if (now) return now;
      if (r.hold) return until(r, () => take(r) ?? (r.hold ? undefined : { ok: true, note: 'the user let you go on' }), holdMs, { ok: true, note: 'the user is still talking: call listen() now' });
      const words = String(text ?? '').trim();
      if (!words) return { ok: true };
      const sid = String(r.seq + 1);
      const audio = speaker ? speaker.speak(words) : Promise.reject(new Error('no voice on this server'));
      audio.catch((err) => log(`[bearclaw-web] qa ${id}: voice for "${words.slice(0, 40)}": ${err?.message ?? err}`));
      r.says.set(sid, { text: words, audio, state: 'queued' });
      r.order.push(sid);
      push(r, { type: 'say', sid, text: words });
      // Back once everything said before this has been heard (or dropped).
      const before = r.order.slice(0, -1);
      const unheard = { ok: true, note: 'not heard yet (the user is away or has not started): call listen() and wait for them' };
      const got = await until(r, () => take(r) ?? (r.hold ? undefined : before.every((b) => r.says.get(b)?.state !== 'queued') ? { ok: true } : undefined), holdMs, unheard);
      // Nobody listening: the listen() this sends it to is a pause, not the end of the talk.
      if (got === unheard) r.away = true;
      return got;
    },

    async show(id, body) {
      const { answering, ...slide } = body ?? {};
      const r = room(id);
      const now = take(r) ?? stale(r, answering);
      if (now) return now;
      if (r.hold) return until(r, () => take(r) ?? (r.hold ? undefined : { ok: true }), holdMs, { ok: true, note: 'the user is still talking: call listen() now' });
      push(r, { type: 'slide', slide });
      return { ok: true };
    },

    async status(id, text) {
      push(room(id), { type: 'status', text: String(text ?? '').slice(0, 200) });
      return { ok: true };
    },

    async listen(id) {
      const r = room(id);
      // The answer is given: the talk goes on without the question's id.
      r.answering = null;
      r.asked = null;
      const now = take(r);
      if (now) return now;
      if (r.cont && !r.hold) {
        r.cont = false;
        return { continue: true };
      }
      push(r, r.away ? { type: 'listening', away: true } : { type: 'listening' });
      r.away = false;
      return until(
        r,
        () => {
          const q = take(r);
          if (q) return q;
          if (r.cont && !r.hold) {
            r.cont = false;
            return { continue: true };
          }
          return undefined;
        },
        holdMs,
        { waiting: true },
      );
    },

    // --- the screen ---------------------------------------------------------------------

    /** A sentence has been heard: the presenter may queue the one after next. */
    played(id, sid) {
      const r = room(id);
      const s = r.says.get(String(sid));
      if (s && s.state === 'queued') s.state = 'played';
      if (s && Number(sid) > r.heard) {
        r.heard = Number(sid);
        save(r, 'state.json', (f) => writeFile(f, `${JSON.stringify({ heard: r.heard })}\n`));
      }
      wake(r);
      return { ok: true };
    },

    /** The user started talking: drop what is queued, hold the presenter until they ask. */
    interrupt(id) {
      const r = room(id);
      for (const sid of r.order) {
        const s = r.says.get(sid);
        if (s?.state === 'queued') s.state = 'skipped';
      }
      r.hold = true;
      r.cont = false;
      push(r, { type: 'interrupt' });
      return { ok: true };
    },

    /** The user's question: the presenter's waiting (or next) call returns it. */
    ask(id, text) {
      const r = room(id);
      r.hold = false;
      r.cont = false;
      r.question = String(text ?? '').trim();
      push(r, { type: 'question', text: r.question });
      return { ok: true };
    },

    /** Go on with the talk (nothing asked, or after an answer). */
    proceed(id) {
      const r = room(id);
      r.hold = false;
      r.cont = true;
      wake(r);
      return { ok: true };
    },

    /** The question nobody has taken yet (the presenter is not calling its tools), if any. */
    pendingQuestion(id) {
      return rooms.get(id)?.question ?? null;
    },

    /** Hand the pending question over some other way (the caller sends it as a message). */
    takeQuestion(id) {
      const r = rooms.get(id);
      return r ? (take(r)?.question ?? null) : null;
    },
  };
}
