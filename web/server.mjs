#!/usr/bin/env node
// bearclaw-web — zero-dependency server for the bearclaw HTTP API + the bundled PWA.
// Config: the shared bearclaw config ($BEARCLAW_CONFIG or ~/.config/bearclaw/config.json).
// See ARCHITECTURE.md.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { configPath, loadConfig, ensurePath, ensureUtf8Locale, resolveBinary } from './lib/config.mjs';
import { run } from './lib/run.mjs';
import { createBearclawCli } from './lib/bearclaw-cli.mjs';
import { createBearclaw } from './lib/bearclaw.mjs';
import { createBackend } from './lib/backends.mjs';
import { createTranscriptReader, locateTranscript } from './lib/transcript.mjs';
import { createSpawner } from './lib/spawn.mjs';
import { createKiller } from './lib/kill.mjs';
import { createAutoNamer, createSpawnNamer } from './lib/autoname.mjs';
import { createGrouper } from './lib/grouping.mjs';
import { createUploader } from './lib/uploads.mjs';
import { SPEECH_KEY_NAME, createKeySource, createSpeaker, createTranscriber } from './lib/transcribe.mjs';
import { createOperator } from './lib/operator.mjs';
import { createCompleter } from './lib/complete.mjs';
import { createFiles } from './lib/files.mjs';
import { createNotes } from './lib/notes.mjs';
import { createSpawnDirsEditor } from './lib/spawn-dirs.mjs';
import { createRepos } from './lib/repos.mjs';
import { createJobs } from './lib/jobs.mjs';
import { createTouchedIndex } from './lib/touched.mjs';
import { createBriefExtractor } from './lib/brief-extract.mjs';
import { createBriefStore, createBriefs, createClaudeAsk, gitInfo } from './lib/briefs.mjs';
import { createStacks } from './lib/stacks.mjs';
import { fetchPeerHost } from './lib/peers.mjs';
import { createDormant } from './lib/dormant.mjs';
import { createMemos } from './lib/memos.mjs';
import { createApi } from './lib/api.mjs';
import { createHttpServer } from './lib/app.mjs';

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const NAME = 'bearclaw-web';
const VERSION = JSON.parse(fs.readFileSync(path.join(ROOT, 'package.json'), 'utf8')).version;

const log = (line) => process.stdout.write(`${line}\n`);
const logError = (err, context = '') =>
  process.stderr.write(`[error]${context ? ` ${context}` : ''} ${err?.stack ?? String(err)}\n`);

ensurePath(process.env);
ensureUtf8Locale(process.env);

let config;
try {
  config = loadConfig({ env: process.env, webRoot: ROOT, log: console });
} catch (err) {
  process.stderr.write(`[bearclaw-web] ${err.message}\n`);
  process.exit(2);
}

const cli = createBearclawCli({ run, bin: config.bearclawBin });
const bearclaw = createBearclaw({ cli, self: config.self, ttlMs: 2000 });
const backend = createBackend({ run, tmux: config.tmux });
const autoNamer = createAutoNamer({
  cli,
  intervalMs: config.autoName.intervalMinutes * 60 * 1000,
  log,
});
// After an unnamed spawn with a first prompt: name that one session once it has replied.
const spawnNamer = config.autoName.enabled
  ? createSpawnNamer({
      cli,
      listSessions: async () => {
        const host = await bearclaw.localHost({ force: true });
        return host.ok ? host.sessions : [];
      },
      log,
    })
  : null;
const uploader = createUploader({
  dir: config.uploads.dir,
  maxBytes: config.uploads.maxMB * 1024 * 1024,
  retentionDays: config.uploads.retentionDays,
  log,
});
// Only the grouping host runs the pass; the bearclaw accessors are bound once the API exists.
let bearclawAccess = null;
const grouper = config.grouping.enabled
  ? createGrouper({
      cli,
      getBearclaw: () => bearclawAccess.buildBearclaw(),
      peekBearclaw: () => bearclawAccess.peekBearclaw(),
      self: config.self,
      intervalMs: config.grouping.intervalMinutes * 60 * 1000,
      log,
    })
  : null;
const claudeBin = resolveBinary('claude', { extraDirs: [path.join(os.homedir(), '.local', 'bin'), path.join(os.homedir(), '.claude', 'local')] });
// Session briefs: GET/PUT always work; background generation only with web.briefs.enabled.
const briefs = createBriefs({
  settings: config.briefs,
  self: config.self,
  store: createBriefStore({ dir: config.briefs.dir }),
  extractor: createBriefExtractor({ locate: (cwd, id) => locateTranscript(cwd, id) }),
  listSessions: async () => {
    const host = await bearclaw.localHost();
    return host.ok ? host.sessions : null;
  },
  ask: createClaudeAsk({ bin: claudeBin, run }),
  git: (cwd) => gitInfo(cwd, { run }),
  log,
});
// Session stacks: the routes always work (the CLI owns the files); the background membership
// sync runs every web.stacks.syncMinutes while a session is in a stack (BEARCLAW_WEB_STACKS=0: off).
const stacks = createStacks({
  cli,
  self: config.self,
  listSessions: async () => {
    const host = await bearclaw.localHost({ force: true });
    if (!host.ok) throw new Error(host.error);
    return host.sessions;
  },
  gateSessions: async () => {
    const host = await bearclaw.localHost();
    return host.ok ? host.sessions : null;
  },
  syncEnabled: config.stacks.sync,
  syncIntervalMs: config.stacks.syncMinutes * 60 * 1000,
  log,
});
// Session recovery: dormant sessions after a reboot (`bearclaw --local restore … --json`).
const dormant = createDormant({ cli, log });
// Voice: an OpenAI key for speech-to-text (voice prompts) and one for text-to-speech (the
// operator's replies) — OPENAI_TEXT_TO_SPEECH_API_KEY, else the transcription key.
const transcribeKey = createKeySource({ run });
const ttsKey = createKeySource({ name: SPEECH_KEY_NAME, run });
const speechKey = async () => (await ttsKey()) ?? (await transcribeKey());
const transcriber = createTranscriber({ getKey: transcribeKey, log });
// Voice memos: segments transcribed with the same key, titled by `claude -p`.
const memos = createMemos({ ...config.memos, transcriber, ask: createClaudeAsk({ bin: claudeBin, run }), log });
const spawner = createSpawner({ run, tmux: config.tmux, launcher: config.claude, launchers: config.launchers });
const operator = createOperator({ config, bearclaw, spawner, backend, log });
const handleApi = createApi({
  config,
  bearclaw,
  backend,
  transcripts: createTranscriptReader(),
  spawner,
  killer: createKiller({ run, tmux: config.tmux, closeIterm: (s) => backend.closeIterm(s) }),
  cli,
  autoNamer,
  spawnNamer,
  uploader,
  transcriber,
  memos,
  speaker: createSpeaker({ getKey: speechKey, voice: config.operator.voice ?? undefined, log }),
  operator,
  files: createFiles({ home: os.homedir(), run, touched: createTouchedIndex(), roots: config.files.roots }),
  completer: createCompleter({ home: os.homedir(), run }),
  notes: createNotes({ config: config.notes, home: os.homedir(), run }),
  // Settings → Start directories: writes through `bearclaw config set`, hot-reloads config.spawnDirs.
  spawnDirs: createSpawnDirsEditor({ config, configFile: config.configFile ?? configPath(process.env), cli }),
  // Settings → Git repos: `bearclaw --local repos …` + `bearclaw config set repos`.
  repos: createRepos({ cli, configFile: config.configFile ?? configPath(process.env), self: config.self }),
  // Jobs screen: `bearclaw --local jobs …` (list / show / run now / enable|disable).
  jobs: createJobs({ cli, self: config.self }),
  briefs,
  stacks,
  dormant,
  grouper,
  name: NAME,
  version: VERSION,
  logError,
});

bearclawAccess = handleApi;

const server = createHttpServer({ handleApi, uiDir: config.uiDir, log, logError });

server.on('error', (err) => {
  logError(err, 'server');
  process.exit(1);
});

server.listen(config.port, config.bind, () => {
  log(`[bearclaw-web] self=${config.self} listening on http://${config.bind}:${config.port}`);
  log(`[bearclaw-web] config=${config.configFile ?? '(none, defaults)'} peers=${Object.keys(config.peers).join(',') || '(none)'}`);
  log(`[bearclaw-web] bearclaw=${config.bearclawBin} tmux=${config.tmux} ui=${config.uiDir ?? '(none)'}`);
  if (config.autoName.enabled) {
    autoNamer.start();
    log(`[bearclaw-web] autoname every ${config.autoName.intervalMinutes}m (bearclaw name --all --apply)`);
  } else {
    log('[bearclaw-web] autoname off (web.autoName.enabled = false)');
  }
  uploader.start();
  // A running operator keeps its workspace current (its ./api helper names this server's port).
  operator.prepare().catch((err) => logError(err, 'operator workspace'));
  operator.start();
  if (config.operator.clearIdleMinutes) log(`[bearclaw-web] operator: /clear after ${config.operator.clearIdleMinutes}m idle`);
  log(`[bearclaw-web] memos ${config.memos.dir} (audio ${config.memos.audioDir})`);
  log(`[bearclaw-web] uploads ${config.uploads.dir} (max ${config.uploads.maxMB} MB, kept ${config.uploads.retentionDays || '∞'} days)`);
  if (grouper) {
    grouper.start();
    log(`[bearclaw-web] grouping every ${config.grouping.intervalMinutes}m (bearclaw group, all hosts)`);
  } else if (config.grouping.host && config.grouping.host !== config.self) {
    log(`[bearclaw-web] grouping: served by ${config.grouping.host}`);
  } else {
    log('[bearclaw-web] grouping off (web.grouping.enabled = false)');
  }
  if (config.briefs.enabled) {
    briefs.start();
    const b = config.briefs;
    log(`[bearclaw-web] briefs on (${b.dir}; ${b.model}, idle ${b.idleMs / 1000}s, every ≥ ${b.minIntervalMs / 60000}m per session, ≤ ${b.maxCallsPerHour}/h)`);
  } else {
    log(`[bearclaw-web] briefs: background generation off (web.briefs.enabled = false); ${config.briefs.dir}`);
  }
  // The sync judges members on other hosts with the merged bearclaw (the warm snapshot when someone
  // is watching, else one fresh build); a sibling started on a peer is found in that peer's list.
  stacks.attach({
    bearclawBody: async () => handleApi.peekBearclaw() ?? (await handleApi.buildBearclaw()),
    peerSessions: async (name) => {
      const url = config.peers[name];
      if (!url) return null;
      const h = await fetchPeerHost(name, url, { timeoutMs: 6000 });
      return h.ok ? h.sessions : null;
    },
  });
  stacks.start();
  log(config.stacks.sync
    ? `[bearclaw-web] stacks: sync every ${config.stacks.syncMinutes}m while a session is in a stack (bearclaw stack sync)`
    : '[bearclaw-web] stacks: background sync off (BEARCLAW_WEB_STACKS=0)');
  if (config.restore.onBoot) {
    log('[bearclaw-web] restore.onBoot: bringing back dormant sessions (bearclaw restore --all)');
    dormant.restoreOnBoot({ markerFile: config.restore.markerFile }).then((r) => {
      if (r?.restored?.length) handleApi.refreshBearclaw();
    });
  }
});

let shuttingDown = false;
function shutdown(signal) {
  if (shuttingDown) return;
  shuttingDown = true;
  log(`[bearclaw-web] ${signal} received, shutting down`);
  autoNamer.stop();
  spawnNamer?.stop();
  grouper?.stop();
  briefs.stop();
  stacks.stop();
  operator.stop();
  uploader.stop();
  handleApi.stop();
  const timer = setTimeout(() => process.exit(0), 3000);
  timer.unref?.();
  server.close(() => {
    log('[bearclaw-web] closed');
    process.exit(0);
  });
  server.closeIdleConnections?.();
}

process.on('SIGTERM', () => shutdown('SIGTERM'));
process.on('SIGINT', () => shutdown('SIGINT'));
process.on('uncaughtException', (err) => logError(err, 'uncaughtException'));
process.on('unhandledRejection', (err) => logError(err, 'unhandledRejection'));
