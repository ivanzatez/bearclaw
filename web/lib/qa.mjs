// QA sessions: a new Claude session (the presenter) presents someone's work to the user live,
// by voice, and they interrupt it with spoken questions it answers from the real code. The
// presenter talks through its MCP tools (bin/qa-mcp.mjs: say / show / listen / status), which
// reach the QA screen at once through lib/qa-live.mjs. This module keeps one folder per QA
// session under the QA dir (config qa.dir, ~/.local/state/bearclaw/qa/<id>): the record
// (qa.json), and the presenter's workspace — its brief (CLAUDE.md), its MCP server (.mcp.json)
// and settings that let it start without a prompt — and it starts the presenter there.
import { randomBytes } from 'node:crypto';
import { promises as fs } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { HttpError } from './http.mjs';

export const QA_ID_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
const LANGS = { hr: 'Croatian (Latin script)', en: 'English' };
/** Fast enough to keep a conversation going; `model` in the request picks another. */
export const DEFAULT_QA_MODEL = 'claude-sonnet-5-5';
const MCP_SCRIPT = fileURLToPath(new URL('../bin/qa-mcp.mjs', import.meta.url));

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
  const model = typeof body?.model === 'string' && /^[A-Za-z0-9._[\]/:-]{1,100}$/.test(body.model) ? body.model : DEFAULT_QA_MODEL;
  const title = (typeof body?.title === 'string' && body.title.trim()) || source?.title || topic.split('\n')[0];
  return { ok: true, title: title.slice(0, 120), topic, source, lang, model };
}

/** The presenter's CLAUDE.md: who it presents to, what, and how it talks (its MCP tools). */
export function presenterBrief(qa) {
  const src = qa.source;
  const id = src?.session?.slice(0, 8);
  const lines = [
    '# QA session: you present, live, by voice',
    '',
    '<!-- Written by bearclaw-web when this QA session started. -->',
    '',
    'You present to the user live, by voice, in a QA session in the bearclaw web app. They see one slide at a time and hear what you say. They can cut in at any moment with a spoken question. Make it feel like a colleague at a whiteboard talking it through with them: short spoken sentences, a slide that changes as you go, the real code on screen when it helps, and real answers when they ask.',
    '',
    qa.topic ? `Topic: ${qa.topic}` : 'Topic: the work of the session below: what it built or decided, why, and how it works.',
  ];
  if (src) {
    lines.push(
      `Source: the Claude session "${src.title || id}" (host ${src.host || 'this one'}, id ${src.session}${src.cwd ? `, in ${src.cwd}` : ''}) did this work. Learn it: \`bearclaw brief ${id}\` (its summary, links and todos), \`bearclaw peek ${id} --lines 150\`, any artifact or document it links (read it), and the code it touched. Never message, interrupt or change that session.`,
    );
  }
  lines.push(
    '',
    '## How you talk: the `bearclaw-qa` tools, nothing else',
    '',
    'The user hears and sees only what goes through these tools. Your ordinary replies reach nobody. So never end your turn: when you have nothing to say, call `listen()`.',
    '',
    `- \`say(text)\`: one to three short spoken sentences. Plain speech: no markdown, code, paths or ids. Speak ${LANGS[qa.lang] ?? LANGS.hr}, and keep English technical words as they are. It returns once the sentence before it has been heard, so keep calling it with the next sentences. That is how the talk flows without pauses.`,
    '- `show(slide)`: the slide for the point you are about to make, before you say it. It takes `title`, and any of `points` (up to 5, short), `code` (up to 18 lines copied from the real file, with `path`, `start` as the line number of its first line, and `highlight` for lines within the snippet, from 1), and `diagram` (up to 8 `nodes` {id, label, note}, `edges` [from, to, label]). One idea per slide.',
    '- `status(text)`: before anything that takes more than a few seconds, one line on what you do ("Reading delegate-task-tool.ts").',
    '- `listen()`: wait for the user. It returns their question, or `continue` (go on), or `waiting` (call it again).',
    '',
    '## The talk',
    '',
    '- First learn the source quickly: a minute, not ten. Say a first sentence early (what you will present) while you prepare the rest.',
    '- Then 6–10 slides, each with two to six sentences of say. Keep moving from one to the next on your own.',
    '- At the end, say that they can ask anything, and call `listen()`.',
    '',
    '## When they interrupt',
    '',
    'Any tool can return `interrupted` with their question and an `id`. Then:',
    '',
    '1. Drop the point you were making. What you had decided to say before you saw the question is dropped anyway: until you call `listen()`, a `say` or `show` without `answering: <id>` is not said.',
    '2. At once, `say` one short sentence that shows you heard it ("Good question: let me open the delegate tool.").',
    '3. If it needs the code, `status` and read the actual code (files, git, gh).',
    '4. `show` what you found: the real snippet or a diagram of it.',
    '5. `say` the answer in a few sentences, then ask whether to go on, and `listen()`. Pass `answering: <id>` in every `say` and `show` of the answer.',
    '6. On `continue`, pick the talk back up where it fits, reshaped by what they asked.',
    '',
    '## Changing things',
    '',
    'When the user asks you to change something (code, a note, a file, a branch), do it: make the change, `show` the diff or the result, and `say` what you did. Change nothing nobody asked for, and never touch the source session.',
  );
  return `${lines.join('\n')}\n`;
}

/** The workspace's MCP config: the bearclaw-qa server for this QA session. */
export function presenterMcp(qa, { node, apiBase }) {
  return { mcpServers: { 'bearclaw-qa': { command: node, args: [MCP_SCRIPT, '--base', apiBase, '--qa', qa.id] } } };
}

/** The workspace's settings: its MCP server on without asking, its tools (writes too) without prompts. */
export function presenterSettings() {
  return {
    enableAllProjectMcpServers: true,
    enabledMcpjsonServers: ['bearclaw-qa'],
    permissions: { allow: ['mcp__bearclaw-qa', 'Bash', 'Read', 'Grep', 'Glob', 'Edit', 'Write'] },
  };
}

/**
 * @param {{ dir: string, spawner?: object, node?: string, apiBase?: string, now?: () => number }} opts
 * @returns {{ dir, list(), get(id), create(body) }}
 */
export function createQa({ dir, spawner = null, node = process.execPath, apiBase = 'http://127.0.0.1:7777/api', now = Date.now }) {
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

    /** A new QA session: its record and workspace, and its presenter started there. */
    async create(body) {
      const check = validateQa(body);
      if (!check.ok) throw new HttpError(check.error, 400);
      const { ok: _ok, ...fields } = check;
      const id = `${slug(fields.title) || 'qa'}-${randomBytes(2).toString('hex')}`;
      const qa = { id, ...fields, presenter: { tmux: `qa-${id}`.slice(0, 40) }, createdAt: now() };
      const f = folder(id);
      await fs.mkdir(path.join(f, '.claude'), { recursive: true });
      await fs.writeFile(path.join(f, 'qa.json'), `${JSON.stringify(qa, null, 1)}\n`);
      await fs.writeFile(path.join(f, 'CLAUDE.md'), presenterBrief(qa));
      await fs.writeFile(path.join(f, '.mcp.json'), `${JSON.stringify(presenterMcp(qa, { node, apiBase }), null, 2)}\n`);
      await fs.writeFile(path.join(f, '.claude', 'settings.json'), `${JSON.stringify(presenterSettings(), null, 2)}\n`);
      if (spawner) {
        await spawner.spawn({ name: qa.presenter.tmux, dir: f, prompt: 'Start the QA session now (CLAUDE.md says how).', nameGiven: true, model: qa.model, harness: 'claude' });
      }
      return qa;
    },
  };
}
