// QA sessions: the record and the presenter's workspace (lib/qa.mjs), and the live room the
// presenter's MCP tools and the QA screen meet in (lib/qa-live.mjs).
import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { createQa, presenterBrief, validateQa } from '../lib/qa.mjs';
import { createQaLive } from '../lib/qa-live.mjs';

const source = { host: 'mac', session: '7fba201d-3b3d-4ebd-a28e-3f88503badfc', title: 'agent-orchestration-subagent', cwd: '/w' };

test('validateQa needs a topic or a source; title, language and model default sensibly', () => {
  assert.equal(validateQa({}).ok, false);
  const v = validateQa({ source, lang: 'xx' });
  assert.deepEqual([v.ok, v.title, v.lang, v.model], [true, 'agent-orchestration-subagent', 'hr', 'claude-sonnet-5-5']);
  assert.equal(validateQa({ topic: 'How the push watcher works\nmore', lang: 'en', model: 'bad model' }).model, 'claude-sonnet-5-5');
});

test('the brief: the source to learn, the tools to talk with, never ending the turn', () => {
  const b = presenterBrief({ id: 'x', title: 'T', topic: '', source, lang: 'en' });
  assert.match(b, /bearclaw brief 7fba201d/);
  assert.match(b, /Speak English/);
  assert.match(b, /never end your turn/);
  for (const tool of ['say(text)', 'show(slide)', 'status(text)', 'listen()']) assert.ok(b.includes(tool), tool);
});

test('create writes the record and the workspace, and starts the presenter there', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'bearclaw-qa-'));
  const spawned = [];
  const qa = createQa({ dir, now: () => 42, node: '/usr/bin/node', apiBase: 'http://127.0.0.1:7777/api', spawner: { spawn: async (o) => spawned.push(o) } });
  const made = await qa.create({ source, topic: 'Subagenti', lang: 'en' });
  const f = path.join(dir, made.id);
  assert.match(made.id, /^agent-orchestration-subagent-[0-9a-f]{4}$/);
  assert.ok(made.presenter.tmux.length <= 40);
  const mcp = JSON.parse(await fs.readFile(path.join(f, '.mcp.json'), 'utf8'));
  assert.deepEqual(mcp.mcpServers['bearclaw-qa'].args.slice(1), ['--base', 'http://127.0.0.1:7777/api', '--qa', made.id]);
  assert.match(mcp.mcpServers['bearclaw-qa'].args[0], /bin\/qa-mcp\.mjs$/);
  assert.equal(JSON.parse(await fs.readFile(path.join(f, '.claude', 'settings.json'), 'utf8')).enableAllProjectMcpServers, true);
  assert.match(await fs.readFile(path.join(f, 'CLAUDE.md'), 'utf8'), /Topic: Subagenti/);
  assert.deepEqual(spawned.map((s) => [s.name, s.dir, s.model, s.nameGiven]), [[made.presenter.tmux, f, 'claude-sonnet-5-5', true]]);
  assert.equal((await qa.get(made.id)).createdAt, 42);
  assert.deepEqual((await qa.list()).map((q) => q.id), [made.id]);
  await assert.rejects(qa.get('../etc'), { status: 404 });
  await assert.rejects(qa.create({}), { status: 400 });
});

const live = () => createQaLive({ speaker: { speak: async (t) => Buffer.from(`mp3:${t}`) }, holdMs: 300, eventsMs: 300 });

test('say: queued with its voice; returns once the sentence before it has been heard', async () => {
  const q = live();
  assert.deepEqual(await q.say('a', 'First.'), { ok: true });
  let second = null;
  const p = q.say('a', 'Second.').then((r) => (second = r));
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(second, null, 'waits while First plays');
  const { events } = await q.events('a', 0);
  assert.deepEqual(events.map((e) => [e.type, e.text]), [['say', 'First.'], ['say', 'Second.']]);
  assert.equal((await q.audio('a', events[0].sid)).toString(), 'mp3:First.');
  q.played('a', events[0].sid);
  assert.deepEqual(await p, { ok: true });
});

test('interrupt drops what is queued and holds the presenter until the question, which it returns', async () => {
  const q = live();
  await q.say('a', 'One.');
  const held = q.say('a', 'Two.');
  q.interrupt('a');
  const blocked = q.show('a', { title: 'Next' });
  q.ask('a', 'Why a tool?');
  const [r1, r2] = await Promise.all([held, blocked]);
  const answers = [r1, r2].filter((r) => r.interrupted);
  assert.deepEqual(answers, [{ interrupted: true, question: 'Why a tool?' }], 'exactly one call takes the question');
  assert.equal(q.pendingQuestion('a'), null);
  const types = (await q.events('a', 0)).events.map((e) => e.type);
  assert.deepEqual(types, ['say', 'say', 'interrupt', 'question']);
});

test('listen: a question, or continue; silence comes back as waiting; events long-poll', async () => {
  const q = live();
  const l = q.listen('a');
  q.proceed('a');
  assert.deepEqual(await l, { continue: true });
  q.ask('a', 'And the credits?');
  assert.deepEqual(await q.listen('a'), { interrupted: true, question: 'And the credits?' });
  assert.deepEqual(await q.listen('a'), { waiting: true });
  const { seq } = await q.events('a', 0);
  const poll = q.events('a', seq);
  await q.status('a', 'Reading x.ts');
  assert.deepEqual((await poll).events.map((e) => [e.type, e.text]), [['status', 'Reading x.ts']]);
  assert.deepEqual((await q.events('a', seq + 1)).events, []);
});

test('an unheard sentence times out into "call listen()" (the user has not started)', async () => {
  const q = live();
  await q.say('a', 'Hello.');
  const r = await q.say('a', 'Anyone?');
  assert.match(r.note, /call listen\(\)/);
});
