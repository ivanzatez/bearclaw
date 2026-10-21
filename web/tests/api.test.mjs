// End-to-end over real HTTP on ephemeral ports: two API servers (laptop + workstation)
// peering with each other, all child processes faked. No network beyond 127.0.0.1.
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { createApi } from '../lib/api.mjs';
import { createHttpServer } from '../lib/app.mjs';
import { createBearclaw } from '../lib/bearclaw.mjs';
import { createBearclawCli } from '../lib/bearclaw-cli.mjs';
import { normalizeConfig } from '../lib/config.mjs';

function listen(server) {
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(`http://127.0.0.1:${server.address().port}`)));
}

function fakeHost(self, sessions) {
  const calls = [];
  const cli = createBearclawCli({ run: async () => ({ stdout: JSON.stringify(sessions), stderr: '' }) });
  const backend = {
    peek: async (s, lines) => (calls.push(['peek', s.session_id, lines]), `screen of ${s.name}`),
    send: async (s, text) => void calls.push(['send', s.session_id, text]),
    keys: async (s, key) => void calls.push(['keys', s.session_id, key]),
  };
  const transcripts = { messages: async () => ({ messages: [{ role: 'user', text: 'hi' }], total: 1, truncated: false, updatedAt: 1 }) };
  const spawner = { spawn: async (req) => (calls.push(['spawn', req]), { name: req.name, dir: req.dir, tmuxSession: req.name, command: 'claude', trusted: false }) };
  const killer = { kill: async (s) => (calls.push(['kill', s.session_id]), { process: 'terminated', terminal: 'tmux-session-killed' }) };
  const autoNamer = {
    lastRun: null,
    runOnce: async (reason) => {
      calls.push(['autoname', reason]);
      autoNamer.lastRun = { at: 1, ms: 1, reason, ok: true, renamed: [{ from: 'app-9d', to: 'fix-login' }], tmux: [], held: [], errors: [] };
      return autoNamer.lastRun;
    },
  };
  cli.rename = async ({ target, title }) => {
    calls.push(['rename', target, title]);
    if (title === 'while waiting') return { ok: false, result: 'held', held: 'waiting', title, message: `${target} is waiting on you — nothing sent (--force overrides)` };
    if (title === 'explode') throw new Error('tmux: boom');
    return { ok: true, result: 'renamed', title, from: 'app-9d', held: null, tmux: { renamed: true, from: 'fw-101010', to: 'fix-login', note: '⧉ fw-101010 → fix-login' }, message: `app-9d → ${title}` };
  };
  cli.harnesses = async () => {
    calls.push(['harnesses']);
    return [
      { id: 'claude', label: 'Claude Code', command: 'claude', installed: true, models: [{ id: 'x', label: 'X' }] },
      { id: 'codex', label: 'Codex', command: 'codex', installed: self === 'workstation', models: [{ id: '', label: 'Default' }] },
    ];
  };
  cli.usage = async ({ refresh }) => {
    calls.push(['usage', refresh]);
    if (self === 'broken') throw new Error('no Claude Code login found');
    return { account: { email: `me@${self}` }, limits: [{ kind: 'session', percent: 25 }], stale: false };
  };
  cli.prs = async ({ refresh }) => {
    calls.push(['prs', refresh]);
    if (self === 'broken') throw new Error('gh not found: install the GitHub CLI and run `gh auth login`');
    return { user: `me-${self}`, stale: false, error: null, columns: [{ id: 'waitingOnMe', title: 'Waiting on me', count: 1 }], prs: [{ column: 'waitingOnMe', repo: 'app', number: 1, local: { repoPath: '/home/tester/app', worktree: '/home/tester/wt/app feat' } }, { column: 'draftMine', repo: 'app', number: 2 }] };
  };
  cli.prCheckout = async (ref) => {
    calls.push(['prCheckout', ref]);
    if (ref === 'acme/gone#9') throw new Error('no checkout of acme/gone under the repos roots (~/Code) — clone it there first');
    const [, owner, repo, number] = /^(.+)\/(.+)#(\d+)$/.exec(ref);
    const p = `/home/tester/wt/${repo}-feat`;
    return { owner, repo, number: Number(number), branch: 'feat', path: p, created: true, contextLine: `You're working on pull request ${ref} in ${p}.` };
  };
  cli.prSeen = async (ref) => {
    calls.push(['prSeen', ref]);
    return { pr: ref, seenAt: 1000, applied: true };
  };
  return { calls, cli, backend, transcripts, spawner, killer, autoNamer, bearclaw: createBearclaw({ cli, self }) };
}

async function startPair(t) {
  const uiDir = fs.mkdtempSync(path.join(os.tmpdir(), 'bearclaw-web-ui-'));
  fs.writeFileSync(path.join(uiDir, 'index.html'), '<p>custom ui</p>');
  t.after(() => fs.rmSync(uiDir, { recursive: true, force: true }));

  const lap = fakeHost('laptop', [{ session_id: 'aaaaaaaa-0000-0000-0000-000000000001', name: 'one', status: 'idle', backend: 'tmux', handle: '%1', updated_at: 1 }]);
  const remote = fakeHost('workstation', [{ session_id: 'bbbbbbbb-0000-0000-0000-000000000002', name: 'two', status: 'waiting', backend: 'tmux', handle: '%2', updated_at: 2 }]);

  // Bind first so each side knows the other's URL.
  const servers = {};
  const urls = {};
  const cfgs = {};
  for (const [name, host] of [['laptop', lap], ['workstation', remote]]) {
    let handle = null;
    servers[name] = createHttpServer({ handleApi: (req, url) => handle(req, url), uiDir });
    urls[name] = await listen(servers[name]);
    host.setHandle = (h) => (handle = h);
    t.after(() => servers[name].close());
  }
  for (const [name, host] of [['laptop', lap], ['workstation', remote]]) {
    cfgs[name] = normalizeConfig(
      {
        self: name,
        hosts: { laptop: { web: urls.laptop }, workstation: { web: urls.workstation } },
        spawnDirs: [{ label: 'Tmp', paths: { [name]: os.tmpdir() } }],
        web: { autoName: { enabled: true } },
      },
      { env: {}, home: '/home/tester' },
    );
    const api = createApi({ config: cfgs[name], ...host, version: '9.9.9' });
    t.after(() => api.stop());
    host.setHandle(api);
  }
  return { lap, remote, urls };
}

async function get(url) {
  const res = await fetch(url);
  return { status: res.status, body: await res.json() };
}
async function post(url, body) {
  const res = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
  return { status: res.status, body: await res.json() };
}

test('health and settings', async (t) => {
  const { urls } = await startPair(t);
  const h = await get(`${urls.laptop}/api/health`);
  assert.equal(h.status, 200);
  assert.equal(h.body.self, 'laptop');
  assert.equal(h.body.version, '9.9.9');
  assert.equal(h.body.apiVersion, 1);
  const s = await get(`${urls.laptop}/api/settings`);
  assert.deepEqual(s.body.hosts, ['laptop', 'workstation']);
  assert.ok(s.body.quickReplies.length > 0);
  assert.deepEqual(s.body.models[0], { id: '', label: 'Default' });
  assert.ok(s.body.models.some((m) => m.id === 'claude-haiku-4-5-20251001'));
});

test('settings exposes web.models when configured', async () => {
  const host = fakeHost('solo', []);
  const config = normalizeConfig({ self: 'solo', web: { models: [{ id: 'claude-sonnet-5', label: 'Sonnet' }, 'claude-opus-5-5'] } }, { env: {}, home: '/home/tester' });
  const api = createApi({ config, ...host, warmBearclaw: false });
  const res = await api({ method: 'GET', headers: {} }, new URL('http://x/api/settings'));
  assert.deepEqual(res.body.models, [
    { id: 'claude-sonnet-5', label: 'Sonnet' },
    { id: 'claude-opus-5-5', label: 'claude-opus-5-5' },
  ]);
});

test('/api/bearclaw merges self + peer; ?local=1 does not fetch peers', async (t) => {
  const { urls } = await startPair(t);
  const all = await get(`${urls.laptop}/api/bearclaw`);
  assert.equal(all.status, 200);
  assert.deepEqual(all.body.hosts.map((h) => h.name), ['laptop', 'workstation']);
  assert.equal(all.body.hosts[1].sessions[0].host, 'workstation');
  assert.equal(all.body.hosts[1].spawnDirs[0].label, 'Tmp', 'peer advertises its own spawnDirs');
  const local = await get(`${urls.workstation}/api/bearclaw?local=1`);
  assert.deepEqual(local.body.hosts.map((h) => h.name), ['workstation']);
});

test('peek/send/keys are served locally or proxied exactly once', async (t) => {
  const { urls, lap, remote } = await startPair(t);
  const peek = await get(`${urls.laptop}/api/hosts/workstation/sessions/bbbbbbbb/peek?lines=50`);
  assert.equal(peek.status, 200);
  assert.equal(peek.body.host, 'workstation');
  assert.equal(peek.body.text, 'screen of two');
  assert.deepEqual(remote.calls.at(-1), ['peek', 'bbbbbbbb-0000-0000-0000-000000000002', 50]);

  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/sessions/aaaaaaaa/send`, { text: 'go' })).status, 200);
  assert.deepEqual(lap.calls.at(-1), ['send', 'aaaaaaaa-0000-0000-0000-000000000001', 'go']);
  assert.equal((await post(`${urls.laptop}/api/hosts/workstation/sessions/bbbbbbbb/keys`, { key: 'Escape' })).status, 200);
  assert.deepEqual(remote.calls.at(-1), ['keys', 'bbbbbbbb-0000-0000-0000-000000000002', 'Escape']);

  const msgs = await get(`${urls.workstation}/api/hosts/laptop/sessions/aaaaaaaa/messages`);
  assert.equal(msgs.status, 200);
  assert.equal(msgs.body.messages[0].text, 'hi');

  // a request that already went through a proxy is never forwarded again
  const chained = await get(`${urls.laptop}/api/hosts/workstation/sessions/bbbbbbbb/peek?local=1`);
  assert.equal(chained.status, 404);
});

test('spawn validates against this host\'s spawnDirs and proxies to peers', async (t) => {
  const { urls, remote } = await startPair(t);
  const res = await post(`${urls.laptop}/api/hosts/workstation/spawn`, { name: 'My Job', prompt: 'hi' });
  assert.equal(res.status, 200);
  assert.equal(res.body.host, 'workstation');
  assert.equal(res.body.name, 'my-job');
  assert.equal(remote.calls.at(-1)[1].dir, fs.realpathSync(os.tmpdir()));
});

test('spawn rejects dirs outside this host\'s spawnDirs (400)', async (t) => {
  const { urls, remote } = await startPair(t);
  const before = remote.calls.length;
  const outside = await post(`${urls.laptop}/api/hosts/workstation/spawn`, { dir: '/' });
  assert.equal(outside.status, 400);
  assert.match(outside.body.error, /not inside one of this host's spawn dirs/);
  const escape = await post(`${urls.laptop}/api/hosts/workstation/spawn`, { dir: path.join(os.tmpdir(), '..', '..') });
  assert.equal(escape.status, 400);
  assert.equal(remote.calls.length, before, 'nothing spawned');
});

test('errors: unknown host/session/route, bad input, wrong method', async (t) => {
  const { urls } = await startPair(t);
  assert.equal((await get(`${urls.laptop}/api/hosts/nope/sessions/x/peek`)).status, 404);
  assert.equal((await get(`${urls.laptop}/api/hosts/laptop/sessions/zzzzzzzzzz/peek`)).status, 404);
  assert.equal((await get(`${urls.laptop}/api/nope`)).status, 404);
  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/sessions/aaaaaaaa/send`, { text: '' })).status, 400);
  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/sessions/aaaaaaaa/keys`, { key: 'C-c' })).status, 400);
  assert.equal((await get(`${urls.laptop}/api/hosts/laptop/sessions/aaaaaaaa/send`)).status, 405);
  assert.equal((await post(`${urls.laptop}/api/bearclaw`, {})).status, 405);
});

test('static UI is served from the configured ui dir', async (t) => {
  const { urls } = await startPair(t);
  const res = await fetch(`${urls.laptop}/`);
  assert.equal(res.status, 200);
  assert.equal(await res.text(), '<p>custom ui</p>');
  assert.equal((await fetch(`${urls.laptop}/missing.js`)).status, 404);
});

test('kill is POST-only, served locally or proxied once, and reports what happened', async (t) => {
  const { urls, lap, remote } = await startPair(t);
  assert.equal((await get(`${urls.laptop}/api/hosts/laptop/sessions/aaaaaaaa/kill`)).status, 405);
  const local = await post(`${urls.laptop}/api/hosts/laptop/sessions/aaaaaaaa/kill`, {});
  assert.equal(local.status, 200);
  assert.deepEqual(local.body, {
    ok: true,
    host: 'laptop',
    id: 'aaaaaaaa-0000-0000-0000-000000000001',
    name: 'one',
    process: 'terminated',
    terminal: 'tmux-session-killed',
  });
  assert.deepEqual(lap.calls.at(-1), ['kill', 'aaaaaaaa-0000-0000-0000-000000000001']);
  const proxied = await post(`${urls.laptop}/api/hosts/workstation/sessions/bbbbbbbb/kill`, {});
  assert.equal(proxied.body.host, 'workstation');
  assert.deepEqual(remote.calls.at(-1), ['kill', 'bbbbbbbb-0000-0000-0000-000000000002']);
  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/sessions/zzzzzzzzzz/kill`, {})).status, 404);
});

test('autoname runs on the target host and shows up in /api/health', async (t) => {
  const { urls, remote } = await startPair(t);
  assert.equal((await get(`${urls.laptop}/api/hosts/workstation/autoname`)).status, 405);
  const res = await post(`${urls.laptop}/api/hosts/workstation/autoname`, {});
  assert.equal(res.status, 200);
  assert.equal(res.body.host, 'workstation');
  assert.deepEqual(res.body.renamed, [{ from: 'app-9d', to: 'fix-login' }]);
  assert.deepEqual(remote.calls.at(-1), ['autoname', 'manual']);
  const h = await get(`${urls.workstation}/api/health`);
  assert.deepEqual(h.body.autoName, { enabled: true, intervalMinutes: 5, lastRun: remote.autoNamer.lastRun });
  assert.equal((await post(`${urls.laptop}/api/hosts/nope/autoname`, {})).status, 404);
});

test('spawn without a name lets the auto-namer name it (only when auto-naming is on)', async (t) => {
  const { urls, remote } = await startPair(t);
  await post(`${urls.laptop}/api/hosts/workstation/spawn`, {});
  assert.equal(remote.calls.at(-1)[1].nameGiven, false);
  await post(`${urls.laptop}/api/hosts/workstation/spawn`, { name: 'job' });
  assert.equal(remote.calls.at(-1)[1].nameGiven, true);
});

test('spawn passes a valid model through and rejects a bad one (400, nothing spawned)', async (t) => {
  const { urls, remote } = await startPair(t);
  const ok = await post(`${urls.laptop}/api/hosts/workstation/spawn`, { model: 'claude-haiku-4-5-20251001' });
  assert.equal(ok.status, 200);
  assert.equal(remote.calls.at(-1)[1].model, 'claude-haiku-4-5-20251001');
  const before = remote.calls.length;
  const bad = await post(`${urls.laptop}/api/hosts/workstation/spawn`, { model: "x'; touch /tmp/pwned" });
  assert.equal(bad.status, 400);
  assert.match(bad.body.error, /model must be/);
  assert.equal(remote.calls.length, before);
});

function spawnReq(body) {
  return import('node:stream').then(({ PassThrough }) => {
    const req = Object.assign(new PassThrough(), { method: 'POST', headers: {} });
    req.end(JSON.stringify(body));
    return req;
  });
}

test('an unnamed spawn with a first prompt schedules a targeted naming pass (auto-naming on only)', async () => {
  const scheduled = [];
  const spawnNamer = { schedule: async (tmux) => void scheduled.push(tmux) };
  const mk = (enabled) => {
    const host = fakeHost('solo', []);
    const config = normalizeConfig({ self: 'solo', web: { autoName: { enabled } }, spawnDirs: [{ path: os.tmpdir() }] }, { env: {}, home: '/home/tester' });
    return createApi({ config, ...host, spawnNamer, warmBearclaw: false });
  };
  const url = new URL('http://x/api/hosts/solo/spawn');
  const on = mk(true);
  const res = await on(await spawnReq({ prompt: 'reply ok' }), url);
  assert.deepEqual(scheduled, [res.body.tmuxSession]);
  await on(await spawnReq({}), url); // no first prompt: nothing to name from yet
  await on(await spawnReq({ name: 'job', prompt: 'x' }), url); // named by hand
  assert.equal(scheduled.length, 1);
  await mk(false)(await spawnReq({ prompt: 'reply ok' }), url);
  assert.equal(scheduled.length, 1, 'auto-naming off: fw-hhmmss fallback, no pass');
});

test('spawn keeps `claude -n <name>` when auto-naming is off', async () => {
  const host = fakeHost('solo', []);
  const config = normalizeConfig({ self: 'solo', web: { autoName: { enabled: false } }, spawnDirs: [{ path: os.tmpdir() }] }, { env: {}, home: '/home/tester' });
  const api = createApi({ config, ...host, warmBearclaw: false });
  const req = Object.assign(new (await import('node:stream')).PassThrough(), { method: 'POST', headers: {} });
  req.end('{}');
  await api(req, new URL('http://x/api/hosts/solo/spawn'));
  assert.equal(host.calls.at(-1)[1].nameGiven, true);
});

test('/api/bearclaw is served from the warm snapshot with snapshotAt', async (t) => {
  const { urls } = await startPair(t);
  const first = await get(`${urls.laptop}/api/bearclaw`);
  assert.equal(typeof first.body.snapshotAt, 'number');
  const again = await get(`${urls.laptop}/api/bearclaw`);
  assert.ok(again.body.snapshotAt >= first.body.snapshotAt);
  assert.deepEqual(again.body.hosts.map((h) => h.name), ['laptop', 'workstation']);
});

test('static files carry a weak ETag and answer 304 to If-None-Match', async (t) => {
  const { urls } = await startPair(t);
  const res = await fetch(`${urls.laptop}/`);
  const etag = res.headers.get('etag');
  assert.match(etag, /^W\/"[0-9a-f]+-[0-9a-f]+"$/);
  await res.text();
  const cached = await fetch(`${urls.laptop}/`, { headers: { 'if-none-match': etag } });
  assert.equal(cached.status, 304);
  assert.equal(cached.headers.get('etag'), etag);
  const other = await fetch(`${urls.laptop}/`, { headers: { 'if-none-match': 'W/"nope"' } });
  assert.equal(other.status, 200);
  await other.text();
});

test('rename is POST-only, validated, served locally or proxied once, and a held session is a 409', async (t) => {
  const { lap, remote, urls } = await startPair(t);
  const base = `${urls.laptop}/api/hosts`;
  assert.equal((await get(`${base}/laptop/sessions/aaaaaaaa/rename`)).status, 405);
  // The session id goes to the CLI, never the fuzzy id from the URL.
  const local = await post(`${base}/laptop/sessions/aaaaaaaa/rename`, { title: '  Fix login  ' });
  assert.equal(local.status, 200);
  assert.equal(local.body.ok, true);
  assert.equal(local.body.result, 'renamed');
  assert.equal(local.body.title, 'Fix login');
  assert.equal(local.body.tmux.to, 'fix-login');
  assert.deepEqual(lap.calls.at(-1), ['rename', 'aaaaaaaa-0000-0000-0000-000000000001', 'Fix login']);

  const proxied = await post(`${base}/workstation/sessions/bbbbbbbb/rename`, { title: 'Docs refresh' });
  assert.equal(proxied.status, 200);
  assert.deepEqual(remote.calls.at(-1), ['rename', 'bbbbbbbb-0000-0000-0000-000000000002', 'Docs refresh']);
  assert.equal(lap.calls.filter((c) => c[0] === 'rename').length, 1, 'proxied, not run on the laptop');

  const held = await post(`${base}/workstation/sessions/bbbbbbbb/rename`, { title: 'while waiting' });
  assert.equal(held.status, 409);
  assert.equal(held.body.result, 'held');
  assert.match(held.body.error, /waiting on you/);

  const failed = await post(`${base}/laptop/sessions/aaaaaaaa/rename`, { title: 'explode' });
  assert.equal(failed.status, 502);
  assert.match(failed.body.error, /rename failed: tmux: boom/);

  for (const title of [undefined, 42, '   ', 'two\nlines', 'x'.repeat(65)]) {
    assert.equal((await post(`${base}/laptop/sessions/aaaaaaaa/rename`, { title })).status, 400, String(title));
  }
  assert.equal((await post(`${base}/laptop/sessions/zzzzzzzzzz/rename`, { title: 'x' })).status, 404);
});

test('bearclaw CLI rename runs `bearclaw rename <id> <title> --json` and resolves a held report (exit 3)', async () => {
  const calls = [];
  let mode = 'ok';
  const run = async (bin, args, opts) => {
    calls.push({ args, opts });
    if (mode === 'held') {
      throw Object.assign(new Error('exit 3'), { code: 3, stdout: JSON.stringify({ ok: false, result: 'held', held: 'waiting', message: 'x is waiting on you — nothing sent' }) });
    }
    if (mode === 'fail') throw Object.assign(new Error('Error: no live session matches "x"'), { code: 1, stdout: '' });
    return { stdout: JSON.stringify({ ok: true, result: 'sent', title: 'T' }), stderr: '' };
  };
  const cli = createBearclawCli({ run, bin: '/x/bearclaw' });
  assert.equal((await cli.rename({ target: 'abc', title: 'T' })).result, 'sent');
  assert.deepEqual(calls[0].args, ['rename', 'abc', 'T', '--json']);
  assert.equal(calls[0].opts.env.NO_COLOR, '1');
  mode = 'held';
  assert.equal((await cli.rename({ target: 'abc', title: 'T' })).held, 'waiting');
  mode = 'fail';
  await assert.rejects(cli.rename({ target: 'abc', title: 'T' }), /no live session/);
});

test('stacks: settings/health advertise them; a server without the stacks module answers 501, also through a peer', async (t) => {
  const { urls } = await startPair(t);
  const s = await get(`${urls.laptop}/api/settings`);
  assert.deepEqual(s.body.stacks, { enabled: false, model: 'sonnet', generate: true });
  const h = await get(`${urls.laptop}/api/health`);
  assert.deepEqual(h.body.stacks, { sync: false, lastSync: null });
  const proxied = await get(`${urls.laptop}/api/hosts/workstation/stacks`);
  assert.equal(proxied.status, 501);
  assert.match(proxied.body.error, /stacks are not available/);
  assert.equal((await get(`${urls.laptop}/api/hosts/laptop/stacks/not-a-stack`)).status, 400);
  // The kill still works (and skips the sync) without it.
  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/sessions/aaaaaaaa/kill`, {})).status, 200);
});

test('usage is GET-only, served locally or proxied once, with ?refresh=1 passed through', async (t) => {
  const { urls, lap, remote } = await startPair(t);
  const here = await get(`${urls.laptop}/api/hosts/laptop/usage`);
  assert.equal(here.status, 200);
  assert.deepEqual([here.body.host, here.body.account.email, here.body.limits[0].percent], ['laptop', 'me@laptop', 25]);
  const there = await get(`${urls.laptop}/api/hosts/workstation/usage?refresh=1`);
  assert.deepEqual([there.body.host, there.body.account.email], ['workstation', 'me@workstation']);
  assert.deepEqual(lap.calls.filter((c) => c[0] === 'usage'), [['usage', false]]);
  assert.deepEqual(remote.calls.filter((c) => c[0] === 'usage'), [['usage', true]]);
  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/usage`, {})).status, 405);
  assert.equal((await get(`${urls.laptop}/api/hosts/nope/usage`)).status, 404);
});

test('prs is GET-only, served locally or proxied, with ?refresh=1 passed through', async (t) => {
  const { urls, lap, remote } = await startPair(t);
  const here = await get(`${urls.laptop}/api/hosts/laptop/prs`);
  assert.equal(here.status, 200);
  assert.deepEqual([here.body.host, here.body.user, here.body.prs[0].column], ['laptop', 'me-laptop', 'waitingOnMe']);
  // Worktrees get "Open in editor" links from the server the browser asked (no ssh alias: none).
  assert.equal(here.body.prs[0].local.editorUrl, 'vscode://file/home/tester/wt/app%20feat');
  assert.equal(here.body.prs[1].local, undefined);
  const there = await get(`${urls.laptop}/api/hosts/workstation/prs?refresh=1`);
  assert.deepEqual([there.body.host, there.body.user], ['workstation', 'me-workstation']);
  assert.equal(there.body.prs[0].local.editorUrl, null);
  assert.deepEqual(lap.calls.filter((c) => c[0] === 'prs'), [['prs', false]]);
  assert.deepEqual(remote.calls.filter((c) => c[0] === 'prs'), [['prs', true]]);
  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/prs`, {})).status, 405);
});

test('prs/checkout and prs/spawn: worktree first, then a session there told about the PR; proxied to peers', async (t) => {
  const { urls, lap, remote } = await startPair(t);
  const co = await post(`${urls.laptop}/api/hosts/laptop/prs/checkout`, { pr: 'acme/app#12' });
  assert.equal(co.status, 200);
  assert.deepEqual([co.body.host, co.body.worktree.path], ['laptop', '/home/tester/wt/app-feat']);
  assert.equal(co.body.worktree.editorUrl, 'vscode://file/home/tester/wt/app-feat');
  assert.equal(lap.calls.filter((c) => c[0] === 'spawn').length, 0);

  const sp = await post(`${urls.laptop}/api/hosts/workstation/prs/spawn`, { pr: 'acme/app#12', prompt: '  Check CI. ', name: 'pr-12', model: 'opus' });
  assert.equal(sp.status, 200);
  assert.equal(sp.body.host, 'workstation');
  assert.equal(sp.body.spawn.tmuxSession, 'pr-12');
  const [, req] = remote.calls.find((c) => c[0] === 'spawn');
  assert.equal(req.dir, '/home/tester/wt/app-feat');
  assert.equal(req.prompt, "You're working on pull request acme/app#12 in /home/tester/wt/app-feat. Check CI.");
  assert.equal(req.model, 'opus');
  assert.equal(lap.calls.filter((c) => c[0] === 'spawn').length, 0);
});

test('prs/spawn: bad input is a 400 before anything is fetched; a failed checkout spawns nothing', async (t) => {
  const { urls, lap } = await startPair(t);
  for (const pr of ['app#12', 'acme/app', '../x#1', 'acme/app#0', 42]) {
    const r = await post(`${urls.laptop}/api/hosts/laptop/prs/spawn`, { pr });
    assert.equal(r.status, 400, String(pr));
  }
  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/prs/spawn`, { pr: 'acme/app#1', model: 'x y' })).status, 400);
  assert.equal(lap.calls.filter((c) => c[0] === 'prCheckout').length, 0);
  const r = await post(`${urls.laptop}/api/hosts/laptop/prs/spawn`, { pr: 'acme/gone#9' });
  assert.equal(r.status, 502);
  assert.match(r.body.error, /clone it there first/);
  assert.equal(lap.calls.filter((c) => c[0] === 'spawn').length, 0);
  assert.equal((await get(`${urls.laptop}/api/hosts/laptop/prs/spawn`)).status, 405);
});

test('prs/seen marks a PR seen on the host it was fetched on', async (t) => {
  const { urls, lap, remote } = await startPair(t);
  const r = await post(`${urls.laptop}/api/hosts/workstation/prs/seen`, { pr: 'acme/app#30' });
  assert.equal(r.status, 200);
  assert.deepEqual([r.body.host, r.body.pr, r.body.seenAt], ['workstation', 'acme/app#30', 1000]);
  assert.deepEqual(remote.calls.filter((c) => c[0] === 'prSeen'), [['prSeen', 'acme/app#30']]);
  assert.equal(lap.calls.filter((c) => c[0] === 'prSeen').length, 0);
  assert.equal((await post(`${urls.laptop}/api/hosts/laptop/prs/seen`, { pr: 'app#30' })).status, 400);
  assert.equal(lap.calls.filter((c) => c[0] === 'prCheckout').length, 0);
});

test('prs: a CLI failure is a 502 with its reason', async () => {
  const host = fakeHost('broken', []);
  const config = normalizeConfig({ self: 'broken' }, { env: {}, home: '/home/tester' });
  const api = createApi({ config, ...host, warmBearclaw: false });
  await assert.rejects(api({ method: 'GET', headers: {} }, new URL('http://x/api/hosts/broken/prs')), (e) => e.status === 502 && /gh auth login/.test(e.message));
});

test('bearclaw CLI prs runs `bearclaw --local prs --json [--refresh]` and surfaces the last error line', async () => {
  const calls = [];
  let stderr = null;
  const run = async (bin, args) => {
    calls.push(args);
    if (stderr) throw Object.assign(new Error('exit 1'), { code: 1, stderr });
    return { stdout: JSON.stringify({ prs: [], stale: false }), stderr: '' };
  };
  const cli = createBearclawCli({ run, bin: '/x/bearclaw' });
  assert.deepEqual(await cli.prs(), { prs: [], stale: false });
  await cli.prs({ refresh: true });
  assert.deepEqual(calls, [['--local', 'prs', '--json'], ['--local', 'prs', '--json', '--refresh']]);
  stderr = 'warning: prs.orgs: 3 is not a name\nError: gh api graphql failed: HTTP 502\n';
  await assert.rejects(cli.prs(), /^BearclawCliError: gh api graphql failed: HTTP 502$/);
  stderr = "error: unrecognized subcommand 'prs'\n";
  await assert.rejects(cli.prs(), (e) => e.missing === true);
});

test('bearclaw CLI prCheckout runs `bearclaw --local pr checkout --json -- <ref>`', async () => {
  const calls = [];
  let stderr = null;
  const run = async (bin, args) => {
    calls.push(args);
    if (stderr) throw Object.assign(new Error('exit 1'), { code: 1, stderr });
    return { stdout: JSON.stringify({ path: '/w', created: true }), stderr: '' };
  };
  const cli = createBearclawCli({ run, bin: '/x/bearclaw' });
  assert.deepEqual(await cli.prCheckout('acme/app#3'), { path: '/w', created: true });
  assert.deepEqual(calls, [['--local', 'pr', 'checkout', '--json', '--', 'acme/app#3']]);
  stderr = 'Error: git fetch --quiet origin feat: fatal: could not read from remote\n';
  await assert.rejects(cli.prCheckout('acme/app#3'), /^BearclawCliError: git fetch --quiet origin feat: fatal/);
  stderr = null;
  await cli.prSeen('acme/app#3');
  assert.deepEqual(calls.at(-1), ['--local', 'pr', 'seen', '--json', '--', 'acme/app#3']);
  stderr = "error: unrecognized subcommand 'seen'\n";
  await assert.rejects(cli.prSeen('acme/app#3'), (e) => e.missing === true);
});

test('harnesses: per host, cached, Claude models from the config, proxied to peers', async (t) => {
  const { urls, lap, remote } = await startPair(t);
  const here = await get(`${urls.laptop}/api/hosts/laptop/harnesses`);
  assert.equal(here.status, 200);
  assert.deepEqual(here.body.harnesses.map((h) => [h.id, h.installed]), [['claude', true], ['codex', false]]);
  assert.ok(here.body.harnesses[0].models.some((m) => m.id === '')); // DEFAULT_MODELS, not the CLI's
  await get(`${urls.laptop}/api/hosts/laptop/harnesses`);
  assert.equal(lap.calls.filter((c) => c[0] === 'harnesses').length, 1);
  const there = await get(`${urls.laptop}/api/hosts/workstation/harnesses`);
  assert.deepEqual([there.body.host, there.body.harnesses[1].installed], ['workstation', true]);
  assert.equal(remote.calls.filter((c) => c[0] === 'harnesses').length, 1);
});

test('usage: a CLI failure is a 502 with its reason', async () => {
  const host = fakeHost('broken', []);
  const config = normalizeConfig({ self: 'broken' }, { env: {}, home: '/home/tester' });
  const api = createApi({ config, ...host, warmBearclaw: false });
  await assert.rejects(api({ method: 'GET', headers: {} }, new URL('http://x/api/hosts/broken/usage')), (e) => e.status === 502 && /no Claude Code login/.test(e.message));
});

test('bearclaw CLI usage runs `bearclaw --local usage --json [--refresh]` and surfaces its error', async () => {
  const calls = [];
  let fail = false;
  const run = async (bin, args) => {
    calls.push(args);
    if (fail) throw Object.assign(new Error('exit 1'), { code: 1, stderr: 'Error: no Claude Code login found (run `claude` and /login)\n' });
    return { stdout: JSON.stringify({ limits: [], stale: false }), stderr: '' };
  };
  const cli = createBearclawCli({ run, bin: '/x/bearclaw' });
  assert.deepEqual(await cli.usage(), { limits: [], stale: false });
  await cli.usage({ refresh: true });
  assert.deepEqual(calls, [['--local', 'usage', '--json'], ['--local', 'usage', '--json', '--refresh']]);
  fail = true;
  await assert.rejects(cli.usage(), /^BearclawCliError: no Claude Code login found/);
});

test('answer types the keys for the pending question, only when it is safe to', async (t) => {
  const id = 'cccccccc-0000-0000-0000-000000000003';
  const host = fakeHost('solo', [{ session_id: id, name: 'q', status: 'waiting', backend: 'tmux', handle: '%3', updated_at: 3 }]);
  const questions = [{ question: 'Merge it?', header: 'Merge', multiSelect: false, options: [{ label: 'Yes', description: '' }, { label: 'No', description: '' }] }];
  let answered = null;
  host.transcripts = { messages: async () => ({ messages: [{ role: 'assistant', kind: 'question', id: 'tq1', questions, text: '', ...answered }], total: 1, truncated: false }) };
  let screen = 'Merge it?\n❯ 1. Yes\n  2. No\nEnter to select · ↑/↓ to navigate';
  host.backend.peek = async () => screen;
  host.backend.type = async (s, text) => void host.calls.push(['type', s.session_id, text]);
  const config = normalizeConfig({ self: 'solo' }, { env: {}, home: '/home/tester' });
  const api = createApi({ config, ...host, warmBearclaw: false, sleep: async () => {} });
  const server = createHttpServer({ handleApi: api, uiDir: os.tmpdir() });
  const base = await listen(server);
  t.after(() => server.close());
  const call = async (body) => (await post(`${base}/api/hosts/solo/sessions/${id}/answer`, body)).status;

  assert.equal(await call({ toolUseId: 'nope', answers: [{ options: [1] }] }), 404);
  assert.equal(await call({ toolUseId: 'tq1', answers: [{ options: [0, 1] }] }), 400);
  screen = 'Merge it?\n  1. Yes\n❯ 2. No\nEnter to select';
  assert.equal(await call({ toolUseId: 'tq1', answers: [{ options: [1] }] }), 409, 'moved in the terminal');
  assert.equal(host.calls.filter((c) => c[0] === 'keys' || c[0] === 'type').length, 0, 'nothing typed yet');
  screen = 'Merge it?\n❯ 1. Yes\n  2. No\nEnter to select';
  assert.equal(await call({ toolUseId: 'tq1', answers: [{ text: 'later' }] }), 200);
  assert.deepEqual(
    host.calls.filter((c) => c[0] === 'keys' || c[0] === 'type').map((c) => c[2]),
    ['Down', 'Down', 'later', 'Enter'],
  );
  answered = { answers: ['later'] };
  assert.equal(await call({ toolUseId: 'tq1', answers: [{ options: [0] }] }), 409, 'already answered');
});

test('/api/push routes reach the push module; settings carry its public key; 501 without one', async () => {
  const host = fakeHost('solo', []);
  const config = normalizeConfig({ self: 'solo' }, { env: {}, home: '/home/tester' });
  const calls = [];
  const push = {
    publicKey: async () => 'BPUBKEY',
    device: async (endpoint) => (calls.push(['device', endpoint]), { subscribed: false }),
    subscribe: async (body) => (calls.push(['subscribe', body]), { subscribed: true }),
    unsubscribe: async (endpoint) => (calls.push(['unsubscribe', endpoint]), { subscribed: false }),
    presence: (endpoint, visible) => (calls.push(['presence', endpoint, visible]), { ok: true }),
    test: async (endpoint) => (calls.push(['test', endpoint]), { ok: true }),
    status: () => ({ devices: 1, watching: true, lastEvent: null }),
  };
  const api = createApi({ config, ...host, push, warmBearclaw: false });
  const server = createHttpServer({ handleApi: api, uiDir: null });
  const base = await listen(server);
  try {
    assert.deepEqual((await get(`${base}/api/settings`)).body.push, { enabled: true, publicKey: 'BPUBKEY' });
    assert.equal((await get(`${base}/api/health`)).body.push.devices, 1);
    assert.equal((await get(`${base}/api/push?endpoint=${encodeURIComponent('https://web.push.apple.com/x')}`)).status, 200);
    assert.equal((await post(`${base}/api/push/subscribe`, { subscription: { endpoint: 'e' } })).body.subscribed, true);
    assert.equal((await post(`${base}/api/push/presence`, { endpoint: 'e', visible: true })).status, 200);
    assert.equal((await post(`${base}/api/push/test`, { endpoint: 'e' })).status, 200);
    assert.equal((await post(`${base}/api/push/unsubscribe`, { endpoint: 'e' })).status, 200);
    assert.equal((await post(`${base}/api/push/test`, {})).status, 400);
    assert.equal((await post(`${base}/api/push/nope`, {})).status, 404);
    assert.deepEqual(calls.map((c) => c[0]), ['device', 'subscribe', 'presence', 'test', 'unsubscribe']);
    assert.deepEqual(calls[2], ['presence', 'e', true]);
  } finally {
    server.close();
  }
  const bare = createApi({ config, ...host, warmBearclaw: false });
  assert.deepEqual((await bare({ method: 'GET', headers: {} }, new URL('http://x/api/settings'))).body.push, { enabled: false });
  await assert.rejects(bare({ method: 'GET', headers: {} }, new URL('http://x/api/push')), /not available/);
});
