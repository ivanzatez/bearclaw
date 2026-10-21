// The one place the web server talks to the `bearclaw` CLI. Everything the server needs
// from the CLI goes through here, so an alternative UI/TUI server can reuse it and the
// contracts (`bearclaw list --json`, `bearclaw rename --json`, `bearclaw name --all --apply`, `bearclaw name <id> --apply`, `bearclaw group` (+ `--rename` / `--move`),
// `bearclaw usage --json`, `bearclaw --local prs --json`, `bearclaw --local pr checkout --json`, `bearclaw config set`, `bearclaw --local repos …`, `bearclaw --local jobs …`, `bearclaw --local stack … --json`, `bearclaw --local restore … --json`) are documented in one
// spot (see ARCHITECTURE.md).
//
// Peek/send/keys are NOT done through the CLI: `bearclaw peek` truncates to terminal width
// and adds a header, so lib/backends.mjs drives tmux / iTerm directly.

export class BearclawCliError extends Error {
  constructor(message, { timedOut = false } = {}) {
    super(message);
    this.name = 'BearclawCliError';
    this.timedOut = timedOut;
  }
}

/** clap's answer when the binary predates `bearclaw stack` (an older CLI on that host). */
const NO_SUBCOMMAND_RE = /unrecognized subcommand|unexpected argument 'stack'|invalid subcommand/i;

/**
 * `run(file, args, opts)` is injected (lib/run.mjs in production, a fake in tests).
 * `bin` is the resolved `bearclaw` binary (lib/config.mjs#resolveBearclawBin).
 */
export function createBearclawCli({ run, bin = 'bearclaw', timeoutMs = 8000 } = {}) {
  if (typeof run !== 'function') throw new TypeError('run must be a function');

  /** Local Claude sessions: the parsed `bearclaw list --json` array. Throws BearclawCliError. */
  async function list() {
    let stdout;
    try {
      ({ stdout } = await run(bin, ['list', '--json'], { timeout: timeoutMs, maxBuffer: 16 * 1024 * 1024 }));
    } catch (err) {
      const killed = err?.killed || err?.signal === 'SIGTERM';
      if (killed) throw new BearclawCliError(`bearclaw list timed out after ${Math.round(timeoutMs / 1000)}s`, { timedOut: true });
      if (err?.code === 'ENOENT') throw new BearclawCliError(`bearclaw binary not found (${bin}) — install it or set "bearclawBin" in the bearclaw config`);
      throw new BearclawCliError(String(err?.message ?? err));
    }
    const text = (stdout ?? '').trim();
    if (!text) return [];
    let parsed;
    try {
      parsed = JSON.parse(text);
    } catch {
      throw new BearclawCliError('bearclaw returned non-JSON output');
    }
    if (!Array.isArray(parsed)) throw new BearclawCliError('bearclaw did not return an array');
    return parsed.filter((s) => s && typeof s === 'object');
  }

  /**
   * The naming pass: `bearclaw name --all --apply` (or `-n name --all` for a dry run).
   * Generates task-shaped names for every session still carrying Claude's cwd+hash name
   * and types `/rename` into them (the CLI holds ones waiting on a prompt). The CLI
   * brings each renamed session's tmux name along (a slug of the title) — one rename
   * path for everything, see docs/architecture.md → "Session titles".
   * Resolves { stdout, stderr } (human output, NO_COLOR); throws BearclawCliError.
   */
  async function nameAll({ dryRun = false, timeoutMs: t = 300 * 1000 } = {}) {
    const args = dryRun ? ['-n', 'name', '--all'] : ['name', '--all', '--apply'];
    try {
      const { stdout, stderr } = await run(bin, args, { timeout: t, env: { ...process.env, NO_COLOR: '1' } });
      return { stdout: stdout ?? '', stderr: stderr ?? '' };
    } catch (err) {
      const killed = err?.killed || err?.signal === 'SIGTERM';
      if (killed) throw new BearclawCliError(`bearclaw name timed out after ${Math.round(t / 1000)}s`, { timedOut: true });
      if (err?.code === 'ENOENT') throw new BearclawCliError(`bearclaw binary not found (${bin})`);
      throw new BearclawCliError(String(err?.message ?? err));
    }
  }

  /**
   * Name one session: `bearclaw name <target> --apply` — the same generator and rename path as
   * `nameAll`, for a session just spawned (lib/autoname.mjs#createSpawnNamer). Output as
   * `nameAll`. Throws BearclawCliError.
   */
  async function nameOne({ target, timeoutMs: t = 120 * 1000 } = {}) {
    try {
      const { stdout, stderr } = await run(bin, ['name', String(target), '--apply'], { timeout: t, env: { ...process.env, NO_COLOR: '1' } });
      return { stdout: stdout ?? '', stderr: stderr ?? '' };
    } catch (err) {
      throw wrap(err, 'bearclaw name', t);
    }
  }

  /**
   * Rename one session: `bearclaw rename <target> <title> --json` — Claude's `/rename` (the
   * title, the source of truth) plus the derived tmux name. Resolves the report
   * ({ ok, result: renamed|sent|held, held, tmux, message, … }); a held session (waiting
   * on a prompt, exit 3) resolves too, with `ok: false`. Throws BearclawCliError otherwise.
   */
  async function rename({ target, title, timeoutMs: t = 20 * 1000 } = {}) {
    const args = ['rename', String(target), String(title), '--json'];
    let stdout;
    try {
      ({ stdout } = await run(bin, args, { timeout: t, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      // Held exits 3 with the report on stdout; anything else is a real failure.
      const report = tryObject(err?.stdout);
      if (report && report.result === 'held') return report;
      throw wrap(err, 'bearclaw rename', t);
    }
    return parseObject(stdout, 'bearclaw rename');
  }

  function tryObject(text) {
    try {
      const v = JSON.parse(String(text ?? '').trim());
      return v && typeof v === 'object' && !Array.isArray(v) ? v : null;
    } catch {
      return null;
    }
  }

  function wrap(err, what, t) {
    const killed = err?.killed || err?.signal === 'SIGTERM';
    if (killed) return new BearclawCliError(`${what} timed out after ${Math.round(t / 1000)}s`, { timedOut: true });
    if (err?.code === 'ENOENT') return new BearclawCliError(`bearclaw binary not found (${bin})`);
    return new BearclawCliError(String(err?.message ?? err));
  }

  function parseObject(stdout, what) {
    try {
      const v = JSON.parse(String(stdout ?? '').trim());
      if (v && typeof v === 'object' && !Array.isArray(v)) return v;
    } catch {
      /* below */
    }
    throw new BearclawCliError(`${what} returned non-JSON output`);
  }

  /**
   * The grouping pass: `bearclaw group --input - --apply --json` with the merged bearclaw
   * (`/api/bearclaw` body) on stdin. Reads sessions and calls `claude -p`; never sends
   * anything to a session. Resolves the parsed report (docs/cli.md → `bearclaw group`).
   */
  async function groupRun({ bearclaw, refresh = false, consolidate = false, timeoutMs: t = 5 * 60 * 1000 } = {}) {
    const args = ['group', '--input', '-', '--apply', '--json'];
    if (refresh) args.push('--refresh');
    if (consolidate) args.push('--consolidate');
    let stdout;
    try {
      ({ stdout } = await run(bin, args, { timeout: t, input: JSON.stringify(bearclaw ?? { hosts: [] }), env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      throw wrap(err, 'bearclaw group', t);
    }
    return parseObject(stdout, 'bearclaw group');
  }

  /** The stored groups (`bearclaw group --cached --json`): no discovery, no model call. */
  async function groupCached({ timeoutMs: t = 10 * 1000 } = {}) {
    let stdout;
    try {
      ({ stdout } = await run(bin, ['group', '--cached', '--json'], { timeout: t, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      throw wrap(err, 'bearclaw group --cached', t);
    }
    return parseObject(stdout, 'bearclaw group --cached');
  }

  /**
   * A change made on the board: `bearclaw group --rename <id> --label <l> --json`,
   * `bearclaw group --move <host/id> (--to <group id> | --label <new group>) --json`,
   * `bearclaw group --create --label <l> --json` or `bearclaw group --delete <id> --json`. Edits the
   * stored groups only (no discovery, no model call); resolves the report like `--cached`.
   */
  async function groupEdit({ op, id, label, host, session, to, timeoutMs: t = 10 * 1000 } = {}) {
    const args = ['group'];
    // `--flag=value`: a label starting with "-" must not read as a flag.
    if (op === 'rename') args.push(`--rename=${id}`, `--label=${label}`);
    else if (op === 'move') args.push(`--move=${host}/${session}`, to ? `--to=${to}` : `--label=${label}`);
    else if (op === 'create') args.push('--create', `--label=${label}`);
    else if (op === 'delete') args.push(`--delete=${id}`);
    else throw new BearclawCliError(`unknown group edit "${op}"`);
    args.push('--json');
    let stdout;
    try {
      ({ stdout } = await run(bin, args, { timeout: t, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      const e = wrap(err, 'bearclaw group', t);
      const why = String(err?.stderr ?? '').replace(/^Error:\s*/, '').trim();
      if (why && !e.timedOut) e.message = why;
      e.refused = !e.timedOut && Boolean(why);
      throw e;
    }
    return parseObject(stdout, 'bearclaw group');
  }

  /**
   * This machine's Claude subscription limits: `bearclaw usage --json` (`--refresh` skips its
   * 60s cache). Resolves the CLI's object; throws BearclawCliError (no login, endpoint down
   * with nothing cached) with the CLI's reason as the message.
   */
  async function usage({ refresh = false, timeoutMs: t = 20 * 1000 } = {}) {
    const args = ['--local', 'usage', '--json'];
    if (refresh) args.push('--refresh');
    let stdout;
    try {
      ({ stdout } = await run(bin, args, { timeout: t, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      const e = wrap(err, 'bearclaw usage', t);
      const why = String(err?.stderr ?? '').replace(/^(bearclaw:\s*)?(Error:\s*)?/i, '').trim();
      if (why && !e.timedOut) e.message = why;
      throw e;
    }
    return parseObject(stdout, 'bearclaw usage');
  }

  /**
   * The GitHub PRs that need you: `bearclaw --local prs --json [--refresh]` (docs/cli.md → Pull
   * requests). A fetch is four `gh` searches (several seconds each); a fresh cache is instant.
   */
  async function prs({ refresh = false, timeoutMs: t = 120 * 1000 } = {}) {
    const args = ['--local', 'prs', '--json'];
    if (refresh) args.push('--refresh');
    let stdout;
    try {
      ({ stdout } = await run(bin, args, { timeout: t, maxBuffer: 16 * 1024 * 1024, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      const e = wrap(err, 'bearclaw prs', t);
      const why = String(err?.stderr ?? '')
        .split('\n')
        .filter((l) => l.trim() && !l.startsWith('warning:'))
        .pop()
        ?.replace(/^(bearclaw:\s*)?(Error:\s*)?/i, '')
        .trim();
      if (why && !e.timedOut) e.message = why;
      e.missing = !e.timedOut && /unrecognized subcommand 'prs'/i.test(String(err?.stderr ?? ''));
      throw e;
    }
    return parseObject(stdout, 'bearclaw prs');
  }

  /**
   * Check a PR out in a worktree (or find the one it is in): `bearclaw --local pr checkout <ref>
   * --json` → `{ owner, repo, number, title, url, headBranch, baseBranch, crossRepository,
   * branch, path, repoPath, created, contextLine }`. Fetches the PR's head: can take a while.
   */
  async function prCheckout(ref, { timeoutMs: t = 4 * 60 * 1000 } = {}) {
    return prCommand('checkout', ref, t);
  }

  /** Mark a PR's @mentions of you seen: `bearclaw --local pr seen <ref> --json` → `{ pr, seenAt, applied }`. */
  async function prSeen(ref, { timeoutMs: t = 30 * 1000 } = {}) {
    return prCommand('seen', ref, t);
  }

  async function prCommand(sub, ref, t) {
    let stdout;
    try {
      ({ stdout } = await run(bin, ['--local', 'pr', sub, '--json', '--', String(ref)], { timeout: t, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      const e = wrap(err, `bearclaw pr ${sub}`, t);
      const why = String(err?.stderr ?? '')
        .split('\n')
        .filter((l) => l.trim() && !l.startsWith('warning:'))
        .pop()
        ?.replace(/^(bearclaw:\s*)?(Error:\s*)?/i, '')
        .trim();
      if (why && !e.timedOut) e.message = why;
      e.missing = !e.timedOut && new RegExp(`unrecognized subcommand '(pr|${sub})'`, 'i').test(String(err?.stderr ?? ''));
      throw e;
    }
    return parseObject(stdout, `bearclaw pr ${sub}`);
  }

  /**
   * The harnesses this machine can spawn (claude, codex, pi) and their model pickers:
   * `bearclaw --local harness list --json` → `[{ id, label, command, installed, models }]`.
   */
  async function harnesses({ timeoutMs: t = 30 * 1000 } = {}) {
    let stdout;
    try {
      ({ stdout } = await run(bin, ['--local', 'harness', 'list', '--json'], { timeout: t, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      throw wrap(err, 'bearclaw harness list', t);
    }
    const rows = JSON.parse(stdout);
    if (!Array.isArray(rows)) throw new Error('bearclaw harness list: expected an array');
    return rows;
  }

  /**
   * Write one config key: `bearclaw --local config set <key> <json>` against `configFile`
   * (BEARCLAW_CONFIG). The CLI owns the config file: it rewrites the raw JSON atomically
   * (tmp + rename), keeps every other key and refuses a file that does not parse.
   * Resolves { stdout, stderr }; throws BearclawCliError.
   */
  async function configSet({ key, value, configFile = null, timeoutMs: t = 10 * 1000 } = {}) {
    const env = { ...process.env, NO_COLOR: '1' };
    if (configFile) env.BEARCLAW_CONFIG = configFile;
    try {
      const { stdout, stderr } = await run(bin, ['--local', 'config', 'set', String(key), JSON.stringify(value)], { timeout: t, env });
      return { stdout: stdout ?? '', stderr: stderr ?? '' };
    } catch (err) {
      throw wrap(err, 'bearclaw config set', t);
    }
  }

  /**
   * `bearclaw --local stack <args…> --json` (session stacks, docs/architecture.md → Session stacks).
   * Resolves the parsed JSON object. Throws BearclawCliError carrying `exitCode`, `report` (a JSON
   * object the CLI printed on stdout before failing — `set` does on a conflict, exit 3),
   * `missing` (the binary has no `stack` command), `timedOut`; its message is the CLI's stderr.
   */
  async function stack(args, { input = null, timeoutMs: t = 20 * 1000 } = {}) {
    const argv = ['--local', 'stack', ...args.map(String), '--json'];
    let stdout;
    try {
      ({ stdout } = await run(bin, argv, { timeout: t, env: { ...process.env, NO_COLOR: '1' }, ...(input != null ? { input } : {}) }));
    } catch (err) {
      const e = wrap(err, `bearclaw stack ${args[0] ?? ''}`.trim(), t);
      const why = String(err?.stderr ?? '').replace(/^Error:\s*/, '').trim();
      if (why && !e.timedOut) e.message = why.split('\n')[0];
      e.exitCode = typeof err?.code === 'number' ? err.code : null;
      e.report = tryObject(err?.stdout);
      e.missing = !e.timedOut && NO_SUBCOMMAND_RE.test(why);
      throw e;
    }
    return parseObject(stdout, `bearclaw stack ${args[0] ?? ''}`.trim());
  }

  /**
   * `bearclaw --local restore <args…> --json` (session recovery, docs/cli.md → Session recovery):
   * the dormant list (no args), a restore (`--` target / `--all`, `--dry-run`) or a forget
   * (`--forget=<t>` / `--forget-all`), `--closed` for the recently closed list. Resolves the parsed object. Throws BearclawCliError carrying
   * `exitCode` (2 ambiguous, 3 no match, 1 a restore with failures), `report` (the JSON the CLI
   * printed before exiting non-zero), `missing` (the binary predates `restore`), `timedOut`.
   */
  async function restore(args = [], { timeoutMs: t = 60 * 1000 } = {}) {
    const argv = ['--local', 'restore', '--json', ...args.map(String)];
    let stdout;
    try {
      ({ stdout } = await run(bin, argv, { timeout: t, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      const e = wrap(err, 'bearclaw restore', t);
      const why = String(err?.stderr ?? '').replace(/^Error:\s*/, '').trim();
      if (why && !e.timedOut) e.message = why.split('\n')[0];
      e.exitCode = typeof err?.code === 'number' ? err.code : null;
      e.report = tryObject(err?.stdout);
      // `--closed` is newer than `restore`: a CLI without it can't serve the closed list.
      e.missing = !e.timedOut && /unrecognized subcommand|unexpected argument '(?:restore|--closed)'|invalid subcommand/i.test(why);
      throw e;
    }
    return parseObject(stdout, 'bearclaw restore');
  }

  /** `bearclaw --local repos …`: errors carry the CLI's reason, `missing` (no `repos` command), `timedOut`. */
  async function reposRun(args, t) {
    let stdout;
    try {
      ({ stdout } = await run(bin, ['--local', 'repos', ...args], { timeout: t, maxBuffer: 16 * 1024 * 1024, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      const e = wrap(err, `bearclaw repos ${args[0] ?? ''}`.trim(), t);
      const why = String(err?.stderr ?? '').replace(/^Error:\s*/, '').trim();
      if (why && !e.timedOut) e.message = why.split('\n').filter((l) => !l.startsWith('warning:')).pop() || why;
      e.missing = !e.timedOut && /unrecognized subcommand 'repos'|unexpected argument '--all'/i.test(why);
      throw e;
    }
    return stdout ?? '';
  }

  /** Every repo under this host's roots: `bearclaw --local repos --json [--all]` (docs/cli.md → Repos). */
  async function repos({ all = false, timeoutMs: t = 30 * 1000 } = {}) {
    const out = await reposRun(['--json', ...(all ? ['--all'] : [])], t);
    try {
      const v = JSON.parse(out.trim() || '[]');
      if (Array.isArray(v)) return v;
    } catch {
      /* below */
    }
    throw new BearclawCliError('bearclaw repos returned non-JSON output');
  }

  /** Fetch + fast-forward: `bearclaw --local repos sync --json [-- names…]` → the results array. */
  async function reposSync(names = [], { timeoutMs: t = 15 * 60 * 1000 } = {}) {
    const out = await reposRun(['sync', '--json', ...(names.length ? ['--', ...names.map(String)] : [])], t);
    try {
      const v = JSON.parse(out.trim() || '[]');
      if (Array.isArray(v)) return v;
    } catch {
      /* below */
    }
    throw new BearclawCliError('bearclaw repos sync returned non-JSON output');
  }

  /** The sync timer: `bearclaw --local repos install-service [--uninstall]`. */
  async function reposService({ uninstall = false, timeoutMs: t = 20 * 1000 } = {}) {
    return reposRun(['install-service', ...(uninstall ? ['--uninstall'] : [])], t);
  }

  /**
   * `bearclaw --local jobs …` (docs/cli.md → Jobs). Errors carry the CLI's reason, `missing` (no
   * `jobs` command: an older CLI), `notFound` (no such job), `busy` (`run` refused: a session of
   * the job is still working), `timedOut`.
   */
  async function jobsRun(args, t) {
    let stdout;
    try {
      ({ stdout } = await run(bin, ['--local', 'jobs', ...args], { timeout: t, maxBuffer: 16 * 1024 * 1024, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      const e = wrap(err, `bearclaw jobs ${args[0] ?? ''}`.trim(), t);
      const why = String(err?.stderr ?? '').replace(/^Error:\s*/, '').trim();
      if (why && !e.timedOut) e.message = why.split('\n').filter((l) => !l.startsWith('warning:')).pop()?.replace(/^Error:\s*/, '') || why;
      e.missing = !e.timedOut && /unrecognized subcommand 'jobs'|invalid subcommand 'jobs'/i.test(why);
      e.notFound = !e.timedOut && /no job `/.test(why);
      e.busy = !e.timedOut && /pass --force/.test(why);
      throw e;
    }
    return stdout ?? '';
  }

  function parseJson(out, what, check) {
    try {
      const v = JSON.parse(String(out).trim());
      if (check(v)) return v;
    } catch {
      /* below */
    }
    throw new BearclawCliError(`${what} returned non-JSON output`);
  }

  /** Every job with its latest run and live sessions: `bearclaw --local jobs --json`. */
  async function jobs({ timeoutMs: t = 30 * 1000 } = {}) {
    const out = await jobsRun(['--json'], t);
    return parseJson(out.trim() || '[]', 'bearclaw jobs', Array.isArray);
  }

  /** One job with all its kept runs (newest first): `bearclaw --local jobs show <name> --json`. */
  async function jobShow(name, { timeoutMs: t = 30 * 1000 } = {}) {
    const out = await jobsRun(['show', '--json', '--', String(name)], t);
    return parseJson(out, 'bearclaw jobs show', (v) => v && typeof v === 'object' && !Array.isArray(v));
  }

  /** Start a run now: `bearclaw --local jobs run [--force] <name>` → its one-line human report. */
  async function jobRunNow(name, { force = false, timeoutMs: t = 60 * 1000 } = {}) {
    const out = await jobsRun(['run', ...(force ? ['--force'] : []), '--', String(name)], t);
    return out.trim();
  }

  /** Pause / resume: `bearclaw --local jobs enable|disable <name>` → its one-line report. */
  async function jobSetEnabled(name, enabled, { timeoutMs: t = 15 * 1000 } = {}) {
    const out = await jobsRun([enabled ? 'enable' : 'disable', '--', String(name)], t);
    return out.trim();
  }

  // `--flag=value` everywhere: a value starting with "-" must not read as a flag.
  const stackList = () => stack(['list']);
  const stackShow = (id) => stack(['show', id]);
  /** Human edit: the markdown on stdin; exit 3 (conflict) when `expectUpdated` is stale. */
  const stackSet = (id, markdown, expectUpdated = null) =>
    stack(['set', id, ...(expectUpdated ? [`--expect-updated=${expectUpdated}`] : [])], { input: markdown });
  const stackRemove = (id) => stack(['rm', id, '-f']);
  /** Rename: the label is a positional, so `--` keeps one starting with "-" out of clap's flags. */
  const stackRename = (id, label) => stack(['rename', id, '--', label]);
  /** The session's stack, created around it when it has none (may call the model: 150 s). */
  const stackEnsure = (sessionId, { label = null } = {}) =>
    stack(['ensure', sessionId, ...(label ? [`--label=${label}`] : [])], { timeoutMs: 150 * 1000 });
  /**
   * Add a session: this host's, or — `host` — one running on another host, its row carried in
   * `input` (an /api/bearclaw-shaped body on stdin, so the CLI never needs ssh for it).
   */
  const stackAdd = (stackId, sessionId, { host = null, input = null } = {}) =>
    stack(['add', stackId, sessionId, ...(host ? ['--on', host] : []), ...(input ? ['--input', '-'] : [])], {
      ...(input ? { input: JSON.stringify(input) } : {}),
    });
  /** Membership sync; with `input` (the merged /api/bearclaw body) members on other hosts are judged too. */
  const stackSync = ({ input = null } = {}) =>
    stack(['sync', ...(input ? ['--input', '-'] : [])], { ...(input ? { input: JSON.stringify(input) } : {}) });

  /** A `--json` command: its parsed object. Throws BearclawCliError with the CLI's last stderr line (`missing`: an older CLI without it). */
  async function jsonCommand(args, what, t) {
    let stdout;
    try {
      ({ stdout } = await run(bin, args, { timeout: t, env: { ...process.env, NO_COLOR: '1' } }));
    } catch (err) {
      const e = wrap(err, what, t);
      const why = String(err?.stderr ?? '')
        .split('\n')
        .filter((l) => l.trim() && !l.startsWith('warning:'))
        .pop()
        ?.replace(/^(bearclaw:\s*)?(Error:\s*)?/i, '')
        .trim();
      if (why && !e.timedOut) e.message = why;
      e.missing = !e.timedOut && /unrecognized subcommand/i.test(String(err?.stderr ?? ''));
      throw e;
    }
    return parseObject(stdout, what);
  }

  /** Claw, the dashboard's operator: `bearclaw --local claw start --json` → `{ session, dir }` (found, or started in tmux). */
  const clawStart = ({ timeoutMs: t = 30 * 1000 } = {}) => jsonCommand(['--local', 'claw', 'start', '--json'], 'bearclaw claw start', t);

  /** Your open Productive tasks: `bearclaw --local tasks --json` → `{ tasks: [...] }`. */
  const tasks = ({ timeoutMs: t = 60 * 1000 } = {}) => jsonCommand(['--local', 'tasks', '--json'], 'bearclaw tasks', t);

  /** The todo: `bearclaw --local todo list --json` → `{ version, nextId, items: [...] }`. */
  const todo = ({ timeoutMs: t = 10 * 1000 } = {}) => jsonCommand(['--local', 'todo', 'list', '--json'], 'bearclaw todo', t);

  /** Change the todo: `add <text> [--task <url>]`, `done <id>`, `rm <id>`; resolves the list after it. */
  async function todoEdit({ op, text, task, id, timeoutMs: t = 10 * 1000 }) {
    const args = op === 'add' ? ['--local', 'todo', 'add', text, ...(task ? ['--task', task] : [])] : ['--local', 'todo', op, String(id)];
    try {
      await run(bin, args, { timeout: t, env: { ...process.env, NO_COLOR: '1' } });
    } catch (err) {
      const e = wrap(err, `bearclaw todo ${op}`, t);
      const why = String(err?.stderr ?? '').trim().split('\n').pop()?.replace(/^(bearclaw:\s*)?(Error:\s*)?/i, '').trim();
      if (why && !e.timedOut) e.message = why;
      throw e;
    }
    return todo();
  }

  return {
    bin,
    list,
    clawStart,
    tasks,
    todo,
    todoEdit,
    nameAll,
    nameOne,
    rename,
    groupRun,
    groupCached,
    groupEdit,
    usage,
    harnesses,
    prs,
    prCheckout,
    prSeen,
    configSet,
    repos,
    reposSync,
    reposService,
    jobs,
    jobShow,
    jobRunNow,
    jobSetEnabled,
    stackList,
    stackShow,
    stackSet,
    stackRemove,
    stackRename,
    stackEnsure,
    stackAdd,
    stackSync,
    restore,
  };
}
