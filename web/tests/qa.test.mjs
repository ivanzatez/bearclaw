// QA sessions: the record and the presenter's first prompt.
import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { createQa, presenterPrompt, validateQa } from '../lib/qa.mjs';

const source = { host: 'mac', session: '7fba201d-3b3d-4ebd-a28e-3f88503badfc', title: 'agent-orchestration-subagent', cwd: '/w' };

test('validateQa needs a topic or a source; the title and language default sensibly', () => {
  assert.equal(validateQa({}).ok, false);
  const v = validateQa({ source, lang: 'xx' });
  assert.deepEqual([v.ok, v.title, v.lang], [true, 'agent-orchestration-subagent', 'hr']);
  assert.equal(validateQa({ topic: 'How the push watcher works\nmore', lang: 'en' }).title, 'How the push watcher works');
});

test('the first prompt carries the c-0 tag, the source to learn and the beat format', () => {
  const p = presenterPrompt({ id: 'x', title: 'T', topic: '', source, lang: 'en' });
  assert.ok(p.startsWith('[qa · c-0] '));
  assert.match(p, /bearclaw brief 7fba201d/);
  assert.match(p, /Speak English/);
  assert.match(p, /```slide\n\{"title"/);
  assert.ok(p.length < 8000, `${p.length} chars: the spawn route caps prompts at 8000`);
});

test('create writes a record with its presenter tmux name; list and get read it back', async () => {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'bearclaw-qa-'));
  const qa = createQa({ dir, now: () => 42 });
  const made = await qa.create({ source, topic: 'Subagenti u AI Agentu' });
  assert.match(made.id, /^agent-orchestration-subagent-[0-9a-f]{4}$/);
  assert.equal(made.presenter.tmux, `qa-${made.id}`.slice(0, 40));
  assert.ok(made.presenter.tmux.length <= 40);
  assert.match(made.prompt, /Topic: Subagenti u AI Agentu/);
  const back = await qa.get(made.id);
  assert.equal(back.prompt, undefined);
  assert.equal(back.createdAt, 42);
  assert.deepEqual((await qa.list()).map((q) => q.id), [made.id]);
  await assert.rejects(qa.get('../etc'), { status: 404 });
  await assert.rejects(qa.create({}), { status: 400 });
  assert.deepEqual(await createQa({ dir: path.join(dir, 'none') }).list(), []);
});
