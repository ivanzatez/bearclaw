#!/usr/bin/env node
// The QA presenter's MCP server (stdio): say / show / listen / status, each a POST to the web
// server's /api/qa/<id>/mcp/<tool>, which holds the call while the user listens or talks
// (lib/qa-live.mjs). Generated into each QA workspace's .mcp.json by lib/qa.mjs:
//   node qa-mcp.mjs --base http://127.0.0.1:7777/api --qa <id>
import process from 'node:process';
import readline from 'node:readline';

const arg = (name) => {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : null;
};
const base = arg('base') ?? 'http://127.0.0.1:7777/api';
const qa = arg('qa');

const SLIDE = {
  type: 'object',
  properties: {
    title: { type: 'string' },
    points: { type: 'array', items: { type: 'string' }, description: 'Up to 5 short points.' },
    code: {
      type: 'object',
      description: 'The real code, copied from the file: up to 18 lines.',
      properties: {
        path: { type: 'string' },
        start: { type: 'integer', description: 'Line number of the first line.' },
        text: { type: 'string' },
        highlight: { type: 'array', items: { type: 'integer' }, description: 'Lines within the snippet to highlight, from 1.' },
      },
      required: ['text'],
    },
    diagram: {
      type: 'object',
      description: 'Boxes and arrows, laid out left to right: up to 8 nodes.',
      properties: {
        nodes: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, label: { type: 'string' }, note: { type: 'string' } }, required: ['id', 'label'] } },
        edges: { type: 'array', items: { type: 'array', items: { type: 'string' }, description: '[from, to, label?]' } },
      },
      required: ['nodes'],
    },
  },
  required: ['title'],
};

const TOOLS = [
  {
    name: 'say',
    description:
      'Say this aloud to the user now: one to three short spoken sentences, plain speech (no markdown, code, paths or ids). Returns when the sentence before it has been heard, so call it again right away with the next sentences. If it returns {interrupted, question}, the user cut in: stop that point and answer the question.',
    inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] },
  },
  {
    name: 'show',
    description: 'Put this slide on the user\'s screen, in order with what you say (it goes up after the sentences said before it). Show a slide before you talk about it.',
    inputSchema: SLIDE,
  },
  {
    name: 'listen',
    description: 'You have nothing more to say right now: wait for the user. Returns {question} when they ask something, {continue: true} when they want you to go on, or {waiting: true} after a while of silence (then call listen() again).',
    inputSchema: { type: 'object', properties: {} },
  },
  {
    name: 'status',
    description: 'One short line on what you are doing right now, shown at once (e.g. "Reading delegate-task-tool.ts"). Use it before anything that takes more than a few seconds.',
    inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] },
  },
];

function reply(text) {
  return { content: [{ type: 'text', text }] };
}

/** A tool's answer, worded for the presenter. */
function words(name, r) {
  if (r?.interrupted) {
    return `The user interrupted you and asked: "${r.question}". Stop the point you were making. Acknowledge it in one short spoken sentence (say), look at the real code first if the question needs it (status while you do), show what you found, then say the answer. Then call listen().`;
  }
  if (name === 'listen') {
    if (r?.continue) return 'The user wants you to go on: continue the talk where it fits, with show and say.';
    return 'Nothing yet. Call listen() again.';
  }
  return r?.note ? `ok (${r.note})` : 'ok';
}

async function call(name, args) {
  if (!qa) return { ...reply('This MCP server was started without --qa <id>.'), isError: true };
  try {
    const res = await fetch(`${base}/qa/${encodeURIComponent(qa)}/mcp/${name}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(args ?? {}),
    });
    const data = await res.json().catch(() => null);
    if (!res.ok) return { ...reply(`The QA screen's server said ${res.status}: ${data?.error ?? 'error'}`), isError: true };
    return reply(words(name, data));
  } catch (err) {
    return { ...reply(`The QA screen's server is unreachable (${err?.message ?? err}).`), isError: true };
  }
}

const send = (msg) => process.stdout.write(`${JSON.stringify(msg)}\n`);

const rl = readline.createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }
  const { id, method, params } = msg;
  if (id === undefined) return; // a notification
  if (method === 'initialize') {
    send({ jsonrpc: '2.0', id, result: { protocolVersion: params?.protocolVersion ?? '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'bearclaw-qa', version: '0.1.0' } } });
  } else if (method === 'tools/list') {
    send({ jsonrpc: '2.0', id, result: { tools: TOOLS } });
  } else if (method === 'tools/call') {
    send({ jsonrpc: '2.0', id, result: await call(params?.name, params?.arguments) });
  } else if (method === 'ping') {
    send({ jsonrpc: '2.0', id, result: {} });
  } else {
    send({ jsonrpc: '2.0', id, error: { code: -32601, message: `unknown method ${method}` } });
  }
});
