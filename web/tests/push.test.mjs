import { test } from 'node:test';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import {
  createIdleProbe,
  createPush,
  encryptPayload,
  generateVapidKeys,
  notificationText,
  sessionEvents,
  validateSubscription,
  vapidAuthorization,
} from '../lib/push.mjs';

const b = (s) => Buffer.from(s, 'base64url');

// RFC 8291 Appendix A.
const RFC = {
  plaintext: 'V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24',
  asPrivate: 'yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw',
  uaPublic: 'BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4',
  salt: 'DGv6ra1nlYgDCS1FRnbzlw',
  auth: 'BTBZMqHH6r4Tts7J_aSIgg',
  header: 'DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8',
  ciphertext: '8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ',
};

/** The receiving side of RFC 8291, written out separately from the sender. */
function decrypt(msg, ua, auth) {
  const salt = msg.subarray(0, 16);
  const idlen = msg[20];
  const asPublic = msg.subarray(21, 21 + idlen);
  const data = msg.subarray(21 + idlen);
  const shared = ua.computeSecret(asPublic);
  const hk = (ikm, s, info, n) => Buffer.from(crypto.hkdfSync('sha256', ikm, s, info, n));
  const ikm = hk(shared, auth, Buffer.concat([Buffer.from('WebPush: info\0'), ua.getPublicKey(), asPublic]), 32);
  const d = crypto.createDecipheriv('aes-128-gcm', hk(ikm, salt, Buffer.from('Content-Encoding: aes128gcm\0'), 16), hk(ikm, salt, Buffer.from('Content-Encoding: nonce\0'), 12));
  d.setAuthTag(data.subarray(-16));
  const padded = Buffer.concat([d.update(data.subarray(0, -16)), d.final()]);
  assert.equal(padded.at(-1), 2);
  return padded.subarray(0, -1).toString();
}

function fakeSubscription(host = 'web.push.apple.com') {
  const ua = crypto.createECDH('prime256v1');
  ua.generateKeys();
  const auth = crypto.randomBytes(16);
  return { ua, auth, sub: { endpoint: `https://${host}/QH9abc`, keys: { p256dh: ua.getPublicKey().toString('base64url'), auth: auth.toString('base64url') } } };
}

test('encryptPayload matches the RFC 8291 test vector', () => {
  const out = encryptPayload(b(RFC.plaintext), { p256dh: RFC.uaPublic, auth: RFC.auth }, { salt: b(RFC.salt), senderPrivateKey: b(RFC.asPrivate) });
  assert.equal(out.toString('base64url'), Buffer.concat([b(RFC.header), b(RFC.ciphertext)]).toString('base64url'));
});

test('encryptPayload round-trips through an independent decrypt', () => {
  const { ua, auth, sub } = fakeSubscription();
  const msg = encryptPayload(JSON.stringify({ title: 'work-42 finished', body: 'Tests are green.' }), sub.keys);
  assert.deepEqual(JSON.parse(decrypt(msg, ua, auth)), { title: 'work-42 finished', body: 'Tests are green.' });
});

test('vapidAuthorization is an ES256 JWT for the push service origin, signed by the VAPID key', () => {
  const keys = generateVapidKeys();
  assert.equal(b(keys.publicKey).length, 65);
  const header = vapidAuthorization({ endpoint: 'https://web.push.apple.com/QH9abc', keys, now: () => 1_700_000_000_000 });
  const m = /^vapid t=([\w-]+)\.([\w-]+)\.([\w-]+), k=([\w-]+)$/.exec(header);
  assert.ok(m, header);
  assert.deepEqual(JSON.parse(b(m[1])), { typ: 'JWT', alg: 'ES256' });
  const claims = JSON.parse(b(m[2]));
  assert.equal(claims.aud, 'https://web.push.apple.com');
  assert.equal(claims.exp, 1_700_000_000 + 12 * 3600);
  assert.match(claims.sub, /^https:\/\//);
  assert.equal(m[4], keys.publicKey);
  const pub = crypto.createPublicKey({ key: { kty: 'EC', crv: 'P-256', x: keys.privateJwk.x, y: keys.privateJwk.y }, format: 'jwk' });
  assert.ok(crypto.verify('sha256', Buffer.from(`${m[1]}.${m[2]}`), { key: pub, dsaEncoding: 'ieee-p1363' }, b(m[3])));
});

test('validateSubscription takes known push services with P-256 keys only', () => {
  const { sub } = fakeSubscription();
  assert.equal(validateSubscription(sub).endpoint, sub.endpoint);
  assert.ok(validateSubscription(fakeSubscription('fcm.googleapis.com').sub));
  assert.throws(() => validateSubscription(fakeSubscription('evil.example.com').sub), /not a known push service/);
  assert.throws(() => validateSubscription({ ...sub, endpoint: 'http://web.push.apple.com/x' }), /not a known push service/);
  assert.throws(() => validateSubscription({ ...sub, keys: { ...sub.keys, auth: 'AAAA' } }), /auth/);
  assert.throws(() => validateSubscription({ ...sub, keys: { ...sub.keys, p256dh: 'AAAA' } }), /p256dh/);
  assert.throws(() => validateSubscription(null), /required/);
});

test('sessionEvents: waiting on any change into waiting, done on busy → idle, nothing for a first sighting', () => {
  const s = (id, status) => ({ host: 'mac', session_id: id, status });
  const first = sessionEvents(new Map(), [s('a', 'busy'), s('b', 'waiting')]);
  assert.deepEqual(first.events, []);
  const second = sessionEvents(first.next, [s('a', 'idle'), s('b', 'busy'), s('c', 'idle')]);
  assert.deepEqual(second.events.map((e) => [e.kind, e.key]), [['done', 'mac/a']]);
  const third = sessionEvents(second.next, [s('a', 'idle'), s('b', 'waiting')]);
  assert.deepEqual(third.events.map((e) => [e.kind, e.key]), [['waiting', 'mac/b']]);
  assert.deepEqual(sessionEvents(third.next, [s('b', 'idle')]).events, []); // waiting → idle: answered elsewhere
});

test('notificationText keeps one plain line', () => {
  assert.equal(notificationText('## Done\n\nAll **tests** pass: see [the PR](https://x).'), 'Done All tests pass: see the PR.');
  assert.equal(notificationText('a'.repeat(200), 20).length, 20);
  assert.equal(notificationText(null), '');
});

test('createIdleProbe reads HIDIdleTime from ioreg (ns → ms), null off macOS or on failure', async () => {
  const run = async () => ({ stdout: '  |   "HIDIdleTime" = 4500000000\n' });
  assert.equal(await createIdleProbe({ run, platform: 'darwin' })(), 4500);
  assert.equal(await createIdleProbe({ run, platform: 'linux' })(), null);
  assert.equal(await createIdleProbe({ run: async () => { throw new Error('no ioreg'); }, platform: 'darwin' })(), null);
});

async function setup(t, overrides = {}) {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), 'bearclaw-push-'));
  t.after(() => fs.rm(dir, { recursive: true, force: true }));
  let sessions = [];
  let clock = 1_000_000;
  const sent = [];
  const push = createPush({
    dir,
    listSessions: async () => sessions,
    lastReply: async (s) => (s.session_id === 'q' ? { kind: 'question', text: 'Which **lane**?' } : { kind: 'assistant', text: 'All green.' }),
    send: async ({ subscription, payload, urgency }) => (sent.push({ endpoint: subscription.endpoint, payload, urgency }), { status: 201 }),
    now: () => clock,
    ...overrides,
  });
  return {
    dir,
    push,
    sent,
    set: (list) => (sessions = list.map(([id, status, extra]) => ({ host: 'mac', session_id: id, name: `work-${id}`, display_title: `work-${id}`, status, ...extra }))),
    advance: (ms) => (clock += ms),
  };
}

test('createPush notifies subscribed devices of waiting and done, with a link to the session', async (t) => {
  const { push, sent, set, dir } = await setup(t);
  const { sub } = fakeSubscription();
  assert.deepEqual(await push.device(sub.endpoint), { subscribed: false });
  assert.equal((await push.subscribe({ subscription: sub })).subscribed, true);
  set([['a', 'busy'], ['q', 'busy']]);
  await push.tick(); // learns the states
  assert.equal(sent.length, 0);
  set([['a', 'idle'], ['q', 'waiting', { waiting_for: 'approval' }]]);
  await push.tick();
  assert.deepEqual(sent.map((s) => [s.payload.title, s.payload.body, s.urgency]), [
    ['work-a finished', 'All green.', 'normal'],
    ['work-q is waiting on you', 'Which lane?', 'high'],
  ]);
  assert.equal(sent[0].payload.url, '/#/s/mac/a');
  assert.equal(sent[0].payload.tag, 'mac/a');
  // Keys and devices persist, private.
  const stat = await fs.stat(path.join(dir, 'vapid.json'));
  assert.equal(stat.mode & 0o777, 0o600);
  const saved = JSON.parse(await fs.readFile(path.join(dir, 'subscriptions.json'), 'utf8'));
  assert.equal(saved.subscriptions[0].endpoint, sub.endpoint);
  assert.ok(saved.subscriptions[0].lastSentAt);
});

test('createPush skips a device whose app is on screen, and done when the device turned it off', async (t) => {
  const { push, sent, set, advance } = await setup(t);
  const { sub } = fakeSubscription();
  await push.subscribe({ subscription: sub, events: { done: false } });
  set([['a', 'busy'], ['b', 'busy']]);
  await push.tick();
  set([['a', 'idle'], ['b', 'waiting']]);
  push.presence(sub.endpoint, true);
  await push.tick();
  assert.equal(sent.length, 0);
  push.presence(sub.endpoint, false);
  set([['a', 'idle'], ['b', 'busy']]);
  await push.tick();
  advance(60_000);
  set([['a', 'idle'], ['b', 'waiting']]);
  await push.tick();
  assert.deepEqual(sent.map((s) => s.payload.title), ['work-b is waiting on you']);
});

test('createPush with awayOnly holds notifications while the Mac is in use', async (t) => {
  let idle = 10_000;
  const { push, sent, set, advance } = await setup(t, { idleMs: async () => idle });
  const { sub } = fakeSubscription();
  await push.subscribe({ subscription: sub });
  set([['a', 'busy']]);
  await push.tick();
  set([['a', 'idle']]);
  await push.tick();
  assert.equal(sent.length, 0);
  idle = 5 * 60_000;
  advance(60_000);
  set([['a', 'busy']]);
  await push.tick();
  set([['a', 'idle']]);
  await push.tick();
  assert.equal(sent.length, 1);
  await push.subscribe({ subscription: sub, awayOnly: false });
  idle = 0;
  advance(60_000);
  set([['a', 'busy']]);
  await push.tick();
  set([['a', 'idle']]);
  await push.tick();
  assert.equal(sent.length, 2);
});

test('createPush drops a subscription the push service says is gone, and test() reports refusals', async (t) => {
  let status = 410;
  const { push, set } = await setup(t, { send: async () => ({ status, text: 'BadJwtToken' }) });
  const { sub } = fakeSubscription();
  await push.subscribe({ subscription: sub, awayOnly: false });
  set([['a', 'busy']]);
  await push.tick();
  set([['a', 'idle']]);
  await push.tick();
  assert.deepEqual(await push.device(sub.endpoint), { subscribed: false });
  await push.subscribe({ subscription: sub });
  status = 403;
  await assert.rejects(push.test(sub.endpoint), /refused it: HTTP 403 BadJwtToken/);
  status = 201;
  assert.deepEqual(await push.test(sub.endpoint), { ok: true });
  await push.unsubscribe(sub.endpoint);
  await assert.rejects(push.test(sub.endpoint), /not subscribed/);
});

test('createPush rejects bad preferences', async (t) => {
  const { push } = await setup(t);
  const { sub } = fakeSubscription();
  await assert.rejects(push.subscribe({ subscription: sub, events: { waiting: 'yes' } }), /events.waiting/);
  await assert.rejects(push.subscribe({ subscription: sub, awayOnly: 1 }), /awayOnly/);
  assert.throws(() => push.presence('', true), /endpoint/);
});
