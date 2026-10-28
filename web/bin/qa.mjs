#!/usr/bin/env node
// A session presenting its own work in a QA session: the presenter's tools as a command, for a
// session that was not started with the bearclaw-qa MCP server (lib/qa.mjs's selfBrief tells it
// how). Each call POSTs to the web server's /api/qa/<id>/mcp/<tool>, which holds it while the user
// listens or talks (lib/qa-live.mjs), and prints what the tool answers.
//   node qa.mjs <qa-id> say "<sentences>"        [--answering <id>]   (or the text on stdin)
//   node qa.mjs <qa-id> show '<slide JSON>'      [--answering <id>]   (or the JSON on stdin)
//   node qa.mjs <qa-id> status "<what you do>"
//   node qa.mjs <qa-id> listen
//   node qa.mjs <qa-id> end                                          (the user is done)
// $BEARCLAW_WEB_API overrides the web server (default http://127.0.0.1:7777/api).
import process from 'node:process';

import { words } from '../lib/qa-words.mjs';

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(`--${name}`);
  if (i < 0) return null;
  const v = args[i + 1] ?? null;
  args.splice(i, 2);
  return v;
};
const base = flag('base') ?? process.env.BEARCLAW_WEB_API ?? 'http://127.0.0.1:7777/api';
const answering = flag('answering');
const [qa, tool, ...rest] = args;

const fail = (msg) => {
  process.stderr.write(`${msg}\n`);
  process.exit(2);
};
if (!qa || !['say', 'show', 'status', 'listen', 'end'].includes(tool)) fail('usage: qa.mjs <qa-id> say|show|status|listen|end [text or slide JSON] [--answering <id>]');

async function input() {
  if (rest.length) return rest.join(' ');
  if (process.stdin.isTTY) return '';
  let s = '';
  for await (const chunk of process.stdin) s += chunk;
  return s.trim();
}

let body = {};
if (tool === 'say' || tool === 'status') {
  const text = await input();
  if (!text) fail(`${tool}: nothing to ${tool === 'say' ? 'say' : 'show'}`);
  body = { text };
} else if (tool === 'show') {
  try {
    body = JSON.parse(await input());
  } catch (err) {
    fail(`show: the slide is not JSON (${err.message})`);
  }
}
if (answering && (tool === 'say' || tool === 'show')) body.answering = answering;

try {
  const res = await fetch(`${base}/qa/${encodeURIComponent(qa)}/mcp/${tool}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await res.json().catch(() => null);
  if (!res.ok) fail(`The QA screen's server said ${res.status}: ${data?.error ?? 'error'}`);
  process.stdout.write(`${words(tool, data, 'cli')}\n`);
} catch (err) {
  fail(`The QA screen's server is unreachable (${err?.message ?? err}).`);
}
