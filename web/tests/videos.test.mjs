// Videos: the folder layout (video.json, scenes.js, narration, slides/) and the player page.
import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { createVideos, parseSlide } from '../lib/videos.mjs';

async function videosDir() {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'bearclaw-videos-'));
  const v = path.join(dir, 'subagents');
  await fs.mkdir(path.join(v, 'slides'), { recursive: true });
  await fs.writeFile(
    path.join(v, 'video.json'),
    JSON.stringify({
      title: 'Subagents',
      session: 'abc-123',
      audio: '../../etc/narration.m4a',
      scenes: [
        { id: 'open', name: 'Open', start: 0, dur: 10, visual: 6, anchors: [0, 1], lines: [{ t0: 0.5, t1: 3, text: 'Hello.' }] },
        { id: 'call', start: 10, dur: 12.5 },
      ],
    }),
  );
  await fs.writeFile(path.join(v, 'narration.m4a'), Buffer.from('m4a-bytes'));
  await fs.writeFile(path.join(v, 'scenes.js'), "SCENES.push(function (kit) { /* </script> */ });\n");
  await fs.writeFile(path.join(v, 'slides', 'why.json'), JSON.stringify({ title: 'Why a tool', points: ['Cache stays warm'], code: 'delegate_task({})' }));
  await fs.writeFile(path.join(v, 'slides', 'half.json'), '{"title": "Hal');
  await fs.mkdir(path.join(dir, 'Not-A-Video'));
  await fs.mkdir(path.join(dir, 'empty'));
  return dir;
}

test('list: every folder with a video.json, with its length and scene count', async () => {
  const videos = createVideos({ dir: await videosDir() });
  const list = await videos.list();
  assert.equal(list.length, 1);
  assert.deepEqual({ ...list[0], updatedAt: 0 }, { id: 'subagents', title: 'Subagents', session: 'abc-123', host: null, duration: 22.5, scenes: 2, updatedAt: 0 });
  assert.deepEqual(await createVideos({ dir: '/nonexistent/videos' }).list(), []);
});

test('get: scenes read leniently, slides that parse, the folder the session writes to', async () => {
  const dir = await videosDir();
  const v = await createVideos({ dir }).get('subagents');
  assert.equal(v.scenes[1].name, 'call');
  assert.equal(v.scenes[1].visual, 12.5);
  assert.deepEqual(v.scenes[0].lines, [{ t0: 0.5, t1: 3, text: 'Hello.' }]);
  assert.equal(v.slidesDir, path.join(dir, 'subagents', 'slides'));
  assert.deepEqual(v.slides.map((s) => s.name), ['why.json']);
  assert.deepEqual(v.slides[0].points, ['Cache stays warm']);
  await assert.rejects(createVideos({ dir }).get('../etc'), { status: 404 });
  await assert.rejects(createVideos({ dir }).get('empty'), { status: 404 });
});

test('audio stays inside the video folder (a path in video.json is cut to its name)', async () => {
  const a = await createVideos({ dir: await videosDir() }).audio('subagents');
  assert.equal(a.type, 'audio/mp4');
  assert.equal(a.bytes.toString(), 'm4a-bytes');
});

test('player: the scenes inline, unable to end their script early', async () => {
  const html = await createVideos({ dir: await videosDir() }).player('subagents');
  assert.match(html, /const VIDEO = \{"scenes":\[\{"id":"open"/);
  assert.match(html, /SCENES\.push\(function \(kit\) \{ \/\* <\\\/script> \*\/ \}\);/);
  assert.equal(html.match(/<\/script>/g).length, 3);
});

test('parseSlide needs a title; points and code are capped', () => {
  assert.equal(parseSlide('{"points":["x"]}'), null);
  assert.equal(parseSlide('not json'), null);
  const s = parseSlide(JSON.stringify({ title: ' T ', points: Array.from({ length: 20 }, (_, i) => i), code: 'x'.repeat(9000) }));
  assert.equal(s.title, 'T');
  assert.equal(s.points.length, 8);
  assert.equal(s.code.length, 4000);
});
