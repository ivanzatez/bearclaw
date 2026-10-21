// Push notifications: Web Push (RFC 8030) to the phones that subscribed in the UI, sent when a
// session starts waiting on the user or finishes its turn. Zero dependencies: the VAPID JWT
// (RFC 8292) and the aes128gcm payload encryption (RFC 8291) are node:crypto.
//
// The server watches its sessions every `intervalMs` while at least one device is subscribed.
// A device whose app is on screen (presence pings) gets nothing; with `awayOnly`, nothing goes
// out while this machine has had keyboard or mouse input in the last `awayMs`.
import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';

import { HttpError } from './http.mjs';

const RECORD_SIZE = 4096;
const MAX_SUBSCRIPTIONS = 20;
/** Push services a subscription endpoint may point at (the server POSTs to it). */
const PUSH_HOSTS = ['push.apple.com', 'googleapis.com', 'mozilla.com', 'notify.windows.com'];
/** A VAPID `sub` claim: a contact URI for the push service. */
const VAPID_SUBJECT = 'https://github.com/ivanzatez/bearclaw';
const PRESENCE_MS = 45 * 1000;
const COOLDOWN_MS = 30 * 1000;
const BODY_MAX = 160;

export const DEFAULT_EVENTS = Object.freeze({ waiting: true, done: true });

const b64url = (buf) => Buffer.from(buf).toString('base64url');
const unb64url = (s) => Buffer.from(String(s), 'base64url');

/** A fresh P-256 key pair for VAPID: `publicKey` is the raw 65-byte point (base64url), the UI's applicationServerKey. */
export function generateVapidKeys() {
  const { publicKey, privateKey } = crypto.generateKeyPairSync('ec', { namedCurve: 'prime256v1' });
  const jwk = publicKey.export({ format: 'jwk' });
  const raw = Buffer.concat([Buffer.from([4]), unb64url(jwk.x), unb64url(jwk.y)]);
  return { publicKey: b64url(raw), privateJwk: privateKey.export({ format: 'jwk' }) };
}

/** The `Authorization` header for one push service origin (RFC 8292). */
export function vapidAuthorization({ endpoint, keys, now = Date.now, subject = VAPID_SUBJECT }) {
  const aud = new URL(endpoint).origin;
  const header = b64url(JSON.stringify({ typ: 'JWT', alg: 'ES256' }));
  const claims = b64url(JSON.stringify({ aud, exp: Math.floor(now() / 1000) + 12 * 3600, sub: subject }));
  const key = crypto.createPrivateKey({ key: keys.privateJwk, format: 'jwk' });
  const sig = crypto.sign('sha256', Buffer.from(`${header}.${claims}`), { key, dsaEncoding: 'ieee-p1363' });
  return `vapid t=${header}.${claims}.${b64url(sig)}, k=${keys.publicKey}`;
}

/**
 * Encrypt a payload for one subscription (RFC 8291, aes128gcm, a single record).
 * `salt` and `senderPrivateKey` are for the RFC's test vector; leave them out otherwise.
 */
export function encryptPayload(payload, { p256dh, auth }, { salt = crypto.randomBytes(16), senderPrivateKey = null } = {}) {
  const uaPublic = unb64url(p256dh);
  const authSecret = unb64url(auth);
  const ecdh = crypto.createECDH('prime256v1');
  if (senderPrivateKey) ecdh.setPrivateKey(senderPrivateKey);
  else ecdh.generateKeys();
  const asPublic = ecdh.getPublicKey();
  const shared = ecdh.computeSecret(uaPublic);
  const hkdf = (ikm, saltBuf, info, len) => Buffer.from(crypto.hkdfSync('sha256', ikm, saltBuf, info, len));
  const keyInfo = Buffer.concat([Buffer.from('WebPush: info\0'), uaPublic, asPublic]);
  const ikm = hkdf(shared, authSecret, keyInfo, 32);
  const cek = hkdf(ikm, salt, Buffer.from('Content-Encoding: aes128gcm\0'), 16);
  const nonce = hkdf(ikm, salt, Buffer.from('Content-Encoding: nonce\0'), 12);
  const cipher = crypto.createCipheriv('aes-128-gcm', cek, nonce);
  // 0x02: the padding delimiter of the last (only) record.
  const body = Buffer.concat([cipher.update(Buffer.concat([Buffer.from(payload), Buffer.from([2])])), cipher.final(), cipher.getAuthTag()]);
  const header = Buffer.alloc(21);
  salt.copy(header, 0);
  header.writeUInt32BE(RECORD_SIZE, 16);
  header[20] = asPublic.length;
  return Buffer.concat([header, asPublic, body]);
}

/** POST one notification to a subscription's push service. Resolves { status }; never throws on an HTTP error. */
export async function sendWebPush({ subscription, payload, keys, urgency = 'normal', ttl = 3600, fetchImpl = fetch, timeoutMs = 10000 }) {
  const res = await fetchImpl(subscription.endpoint, {
    method: 'POST',
    headers: {
      authorization: vapidAuthorization({ endpoint: subscription.endpoint, keys }),
      'content-encoding': 'aes128gcm',
      'content-type': 'application/octet-stream',
      ttl: String(ttl),
      urgency,
    },
    body: encryptPayload(JSON.stringify(payload), subscription.keys),
    signal: AbortSignal.timeout(timeoutMs),
  });
  return { status: res.status, text: res.ok ? '' : await res.text().catch(() => '') };
}

/** A subscription from the browser's PushSubscription.toJSON(), checked; throws HttpError 400. */
export function validateSubscription(sub) {
  if (!sub || typeof sub !== 'object') throw new HttpError('subscription is required', 400);
  let url;
  try {
    url = new URL(sub.endpoint);
  } catch {
    throw new HttpError('subscription.endpoint must be a URL', 400);
  }
  if (url.protocol !== 'https:' || !PUSH_HOSTS.some((h) => url.hostname === h || url.hostname.endsWith(`.${h}`))) {
    throw new HttpError(`subscription.endpoint is not a known push service: ${url.hostname}`, 400);
  }
  const p256dh = sub.keys?.p256dh;
  const auth = sub.keys?.auth;
  if (typeof p256dh !== 'string' || unb64url(p256dh).length !== 65 || unb64url(p256dh)[0] !== 4) throw new HttpError('subscription.keys.p256dh must be a P-256 public key', 400);
  if (typeof auth !== 'string' || unb64url(auth).length !== 16) throw new HttpError('subscription.keys.auth must be 16 bytes', 400);
  return { endpoint: url.href, keys: { p256dh, auth } };
}

function validatePrefs(body, current = {}) {
  const events = { ...DEFAULT_EVENTS, ...current.events };
  if (body.events != null) {
    if (typeof body.events !== 'object') throw new HttpError('events must be an object { waiting, done }', 400);
    for (const k of Object.keys(DEFAULT_EVENTS)) {
      if (body.events[k] == null) continue;
      if (typeof body.events[k] !== 'boolean') throw new HttpError(`events.${k} must be true or false`, 400);
      events[k] = body.events[k];
    }
  }
  let awayOnly = current.awayOnly ?? true;
  if (body.awayOnly != null) {
    if (typeof body.awayOnly !== 'boolean') throw new HttpError('awayOnly must be true or false', 400);
    awayOnly = body.awayOnly;
  }
  return { events, awayOnly };
}

/** One line of a reply for a notification body: no markdown marks, whitespace collapsed, capped. */
export function notificationText(text, max = BODY_MAX) {
  const line = String(text ?? '')
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/[*_`#>]+/g, '')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\s+/g, ' ')
    .trim();
  return line.length > max ? `${line.slice(0, max - 1).trimEnd()}…` : line;
}

/**
 * What changed between two polls: `waiting` when a session starts waiting on the user, `done`
 * when a busy session goes idle. A session seen for the first time is only remembered.
 */
export function sessionEvents(prev, sessions) {
  const events = [];
  const next = new Map();
  for (const s of sessions) {
    if (!s?.session_id) continue;
    const key = `${s.host}/${s.session_id}`;
    next.set(key, s.status);
    const before = prev.get(key);
    if (before == null || before === s.status) continue;
    if (s.status === 'waiting') events.push({ kind: 'waiting', key, session: s });
    else if (before === 'busy' && s.status === 'idle') events.push({ kind: 'done', key, session: s });
  }
  return { events, next };
}

/** Seconds since the last keyboard or mouse input on this Mac (`ioreg`), cached briefly; null when unknown. */
export function createIdleProbe({ run, platform = process.platform, cacheMs = 5000, now = Date.now }) {
  let cached = null;
  return async function idleMs() {
    if (platform !== 'darwin') return null;
    if (cached && now() - cached.at < cacheMs) return cached.value;
    let value = null;
    try {
      const { stdout } = await run('ioreg', ['-c', 'IOHIDSystem', '-d', '4', '-r', '-k', 'HIDIdleTime'], { timeout: 3000 });
      const m = /"HIDIdleTime"\s*=\s*(\d+)/.exec(stdout);
      if (m) value = Math.round(Number(m[1]) / 1e6);
    } catch {
      value = null;
    }
    cached = { at: now(), value };
    return value;
  };
}

/**
 * @param deps
 *   dir           where vapid.json and subscriptions.json live
 *   listSessions  async () => sessions (each with host, session_id, status, …) or null on failure
 *   lastReply     async (session) => the newest assistant message { kind, text } or null
 *   idleMs        async () => ms since this machine's last keyboard/mouse input, or null (unknown: treated as away)
 *   send          sendWebPush (tests inject a fake)
 *   intervalMs, awayMs, now, log
 */
export function createPush({
  dir,
  listSessions,
  lastReply = async () => null,
  idleMs = async () => null,
  send = sendWebPush,
  intervalMs = 5000,
  awayMs = 2 * 60 * 1000,
  now = Date.now,
  log = () => {},
}) {
  const keysFile = path.join(dir, 'vapid.json');
  const subsFile = path.join(dir, 'subscriptions.json');
  let keys = null;
  let subs = null; // [{ endpoint, keys, events, awayOnly, createdAt, lastSentAt, lastError }]
  const presence = new Map(); // endpoint -> { visible, at }
  const sentAt = new Map(); // `${kind} ${key}` -> ms (cooldown)
  let prev = null; // Map key -> status, null until the first poll after someone subscribed
  let timer = null;
  let ticking = null;
  let lastEvent = null;

  async function writeJson(file, value, mode) {
    await fs.mkdir(dir, { recursive: true, mode: 0o700 });
    const tmp = `${file}.${process.pid}.tmp`;
    await fs.writeFile(tmp, `${JSON.stringify(value, null, 2)}\n`, { mode });
    await fs.rename(tmp, file);
  }

  async function loadKeys() {
    if (keys) return keys;
    try {
      const k = JSON.parse(await fs.readFile(keysFile, 'utf8'));
      if (typeof k?.publicKey === 'string' && k?.privateJwk?.d) keys = k;
    } catch {
      /* first run */
    }
    if (!keys) {
      keys = generateVapidKeys();
      await writeJson(keysFile, keys, 0o600);
      log('[push] generated VAPID keys');
    }
    return keys;
  }

  async function loadSubs() {
    if (subs) return subs;
    try {
      const list = JSON.parse(await fs.readFile(subsFile, 'utf8'))?.subscriptions;
      subs = Array.isArray(list) ? list.filter((s) => typeof s?.endpoint === 'string' && s.keys) : [];
    } catch {
      subs = [];
    }
    return subs;
  }

  const saveSubs = () => writeJson(subsFile, { subscriptions: subs }, 0o600);
  const view = (s) => (s ? { subscribed: true, events: s.events, awayOnly: s.awayOnly, lastSentAt: s.lastSentAt ?? null, lastError: s.lastError ?? null } : { subscribed: false });

  async function deliver(sub, payload, urgency) {
    try {
      const r = await send({ subscription: sub, payload, keys: await loadKeys(), urgency });
      if (r.status === 404 || r.status === 410) {
        // The browser dropped this subscription (app removed, permission revoked).
        subs = subs.filter((s) => s.endpoint !== sub.endpoint);
        await saveSubs();
        log(`[push] subscription gone (${r.status}), removed`);
        return { ok: false, status: r.status, error: 'subscription expired' };
      }
      sub.lastSentAt = now();
      sub.lastError = r.status >= 200 && r.status < 300 ? null : `HTTP ${r.status} ${r.text}`.trim();
      if (sub.lastError) log(`[push] ${sub.lastError}`);
      return { ok: !sub.lastError, status: r.status, error: sub.lastError };
    } catch (err) {
      sub.lastError = err?.message ?? String(err);
      log(`[push] send failed: ${sub.lastError}`);
      return { ok: false, status: 0, error: sub.lastError };
    }
  }

  async function notification(ev) {
    const s = ev.session;
    const name = s.display_title || s.name || s.session_id.slice(0, 8);
    let reply = null;
    try {
      reply = await lastReply(s);
    } catch {
      reply = null;
    }
    const text = notificationText(reply?.text);
    const body =
      ev.kind === 'waiting'
        ? reply?.kind === 'question' && text
          ? text
          : s.waiting_for
            ? `Waiting for ${notificationText(s.waiting_for, 80)}`
            : 'Waiting on you'
        : text || 'Finished its turn';
    return {
      title: ev.kind === 'waiting' ? `${name} is waiting on you` : `${name} finished`,
      body,
      tag: ev.key,
      url: `/#/s/${encodeURIComponent(s.host)}/${encodeURIComponent(s.session_id)}`,
    };
  }

  async function tick() {
    if (ticking) return ticking;
    ticking = (async () => {
      await loadSubs();
      if (!subs.length) {
        prev = null;
        return;
      }
      const sessions = await listSessions();
      if (!Array.isArray(sessions)) return;
      // The first poll after someone subscribed only learns the current states.
      const { events, next } = sessionEvents(prev ?? new Map(), sessions);
      const first = !prev;
      prev = next;
      if (first) return;
      for (const [k, at] of sentAt) if (now() - at >= COOLDOWN_MS) sentAt.delete(k);
      const due = events.filter((ev) => {
        const k = `${ev.kind} ${ev.key}`;
        if (now() - (sentAt.get(k) ?? 0) < COOLDOWN_MS) return false;
        sentAt.set(k, now());
        return true;
      });
      if (!due.length) return;
      const idle = await idleMs();
      const atMachine = idle != null && idle < awayMs;
      let changed = false;
      for (const ev of due) {
        const payload = await notification(ev);
        for (const sub of [...subs]) {
          if (!sub.events?.[ev.kind]) continue;
          const p = presence.get(sub.endpoint);
          if (p?.visible && now() - p.at < PRESENCE_MS) continue;
          if (sub.awayOnly && atMachine) continue;
          await deliver(sub, payload, ev.kind === 'waiting' ? 'high' : 'normal');
          changed = true;
        }
        lastEvent = { at: now(), kind: ev.kind, key: ev.key };
      }
      if (changed && subs.length) await saveSubs();
    })()
      .catch((err) => log(`[push] watch failed: ${err?.message ?? err}`))
      .finally(() => {
        ticking = null;
      });
    return ticking;
  }

  return {
    async publicKey() {
      return (await loadKeys()).publicKey;
    },

    async device(endpoint) {
      await loadSubs();
      return view(subs.find((s) => s.endpoint === endpoint));
    },

    /** Subscribe a device, or update its preferences ({ subscription, events?, awayOnly? }). */
    async subscribe(body) {
      const sub = validateSubscription(body?.subscription);
      await loadSubs();
      const existing = subs.find((s) => s.endpoint === sub.endpoint);
      const prefs = validatePrefs(body, existing);
      if (existing) Object.assign(existing, sub, prefs);
      else {
        subs.push({ ...sub, ...prefs, createdAt: now(), lastSentAt: null, lastError: null });
        subs = subs.slice(-MAX_SUBSCRIPTIONS);
        log(`[push] device subscribed (${subs.length} in all)`);
      }
      await saveSubs();
      return view(subs.find((s) => s.endpoint === sub.endpoint));
    },

    async unsubscribe(endpoint) {
      await loadSubs();
      const before = subs.length;
      subs = subs.filter((s) => s.endpoint !== endpoint);
      presence.delete(endpoint);
      if (subs.length !== before) await saveSubs();
      return { subscribed: false };
    },

    /** The app on that device is on screen (or not): nothing is sent to it while it is. */
    presence(endpoint, visible) {
      if (typeof endpoint !== 'string' || !endpoint) throw new HttpError('endpoint is required', 400);
      presence.set(endpoint, { visible: visible === true, at: now() });
      return { ok: true };
    },

    /** Send a test notification to one device now, whatever its preferences. */
    async test(endpoint) {
      await loadSubs();
      const sub = subs.find((s) => s.endpoint === endpoint);
      if (!sub) throw new HttpError('this device is not subscribed', 404);
      const r = await deliver(sub, { title: 'bearclaw', body: 'Notifications work. You will hear from your sessions here.', tag: 'bearclaw-test', url: '/#/' }, 'high');
      await saveSubs().catch(() => {});
      if (!r.ok) throw new HttpError(`the push service refused it: ${r.error}`, 502);
      return { ok: true };
    },

    status() {
      return { devices: subs?.length ?? 0, watching: Boolean(timer), lastEvent };
    },

    tick,

    start() {
      if (timer) return;
      timer = setInterval(() => tick(), intervalMs);
      timer.unref?.();
    },

    stop() {
      clearInterval(timer);
      timer = null;
    },
  };
}
