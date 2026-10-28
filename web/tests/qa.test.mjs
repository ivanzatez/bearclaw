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
  assert.match(b, /without `answering: <id>` is not said/);
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
  assert.deepEqual(answers, [{ interrupted: true, question: 'Why a tool?', id: 'q1' }], 'exactly one call takes the question');
  assert.equal(q.pendingQuestion('a'), null);
  const types = (await q.events('a', 0)).events.map((e) => e.type);
  assert.deepEqual(types, ['say', 'say', 'interrupt', 'question']);
});

test('the rest of a batch decided before the question is dropped; the answer, with its id, is said', async () => {
  const q = live();
  await q.say('a', 'Old one.');
  const blocked = q.say('a', 'Old two.');
  q.interrupt('a');
  q.ask('a', 'Why a tool?');
  const got = await blocked;
  assert.deepEqual(got, { interrupted: true, question: 'Why a tool?', id: 'q1' });
  // The calls the presenter had already decided, run right after: not said.
  assert.deepEqual(await q.say('a', 'Old three.'), { dropped: true, question: 'Why a tool?', id: 'q1' });
  assert.equal((await q.show('a', { title: 'Old slide' })).dropped, true);
  // Its answer, passing the id, is.
  assert.deepEqual(await q.show('a', { title: 'Why a tool', answering: 'q1' }), { ok: true });
  assert.deepEqual(await q.say('a', 'Because the cache stays warm.', 'q1'), { ok: true });
  const after = (await q.events('a', 0)).events.filter((e) => e.seq > 4);
  assert.deepEqual(after.map((e) => [e.type, e.text ?? e.slide?.title]), [['slide', 'Why a tool'], ['say', 'Because the cache stays warm.']]);
  assert.equal(after[0].slide.answering, undefined, 'the id is not part of the slide');
  // listen() ends the answer: the talk goes on without the id.
  const l = q.listen('a');
  q.proceed('a');
  assert.deepEqual(await l, { continue: true });
  q.played('a', after[1].sid);
  assert.deepEqual(await q.say('a', 'Back to the talk.'), { ok: true });
});

test('listen: a question, or continue; silence comes back as waiting; events long-poll', async () => {
  const q = live();
  const l = q.listen('a');
  q.proceed('a');
  assert.deepEqual(await l, { continue: true });
  q.ask('a', 'And the credits?');
  assert.deepEqual(await q.listen('a'), { interrupted: true, question: 'And the credits?', id: 'q1' });
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

test('a room is saved: after a restart the screen gets what was said and how far it was heard', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'bearclaw-qa-live-'));
  await fs.mkdir(path.join(dir, 'talk-0001'));
  const speak = async (t) => Buffer.from(`mp3:${t}`);
  const a = createQaLive({ speaker: { speak }, dir, holdMs: 200, eventsMs: 50 });
  await a.show('talk-0001', { title: 'Intro' });
  await a.say('talk-0001', 'One.');
  const first = (await a.events('talk-0001', 0)).events.find((e) => e.type === 'say');
  a.played('talk-0001', first.sid);
  await a.say('talk-0001', 'Two.');
  await new Promise((r) => setTimeout(r, 50)); // the writes land

  const b = createQaLive({ speaker: { speak }, dir, holdMs: 200, eventsMs: 50 });
  const back = await b.events('talk-0001', 0);
  assert.deepEqual(back.events.map((e) => e.type), ['slide', 'say', 'say']);
  assert.equal(back.heard, Number(first.sid));
  assert.equal(back.seq, 3);
  // Its voice is made again on demand; the next event continues the numbering.
  assert.equal((await b.audio('talk-0001', '3')).toString(), 'mp3:Two.');
  await b.status('talk-0001', 'Reading x.ts');
  assert.equal((await b.events('talk-0001', 3)).events[0].seq, 4);
});

test('a listen() after nobody heard the last sentence is a pause (away), not the end of the talk', async () => {
  const q = live();
  await q.say('a', 'Hello.');
  await q.say('a', 'Anyone?'); // times out unheard
  const l = q.listen('a');
  const { events } = await q.events('a', 2);
  assert.deepEqual(events.map((e) => [e.type, e.away]), [['listening', true]]);
  q.proceed('a');
  await l;
  const again = q.listen('a');
  const next = await q.events('a', 3);
  assert.equal(next.events[0].away, undefined, 'only the first listen after it');
  q.proceed('a');
  await again;
});
