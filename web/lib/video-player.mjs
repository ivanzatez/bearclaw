// The video player page (GET /api/videos/<id>/player): the stage a video's scenes draw on, run in
// a sandboxed iframe by the Videos screen. The parent owns the clock (the narration's own time)
// and posts it here; this page only draws.
//   parent → here   { type: 'frame', t }       the film at t seconds
//                   { type: 'slide', slide }   an answer slide over the film ({ title, points, code }), null hides it
//   here → parent   { type: 'ready' }
// Scenes are pure functions of their own time (the Motion film's contract): `scenes.js` pushes
// modules onto SCENES, each `function (kit) { kit.scene(id, { init(kit), render(t) }) }`. A spoken
// line starts at its scene's `anchors[k]` (visual seconds); time between anchors is stretched.

/** JSON for an inline <script>: no `</script>` can end it early. */
const inline = (v) => JSON.stringify(v).replace(/</g, '\\u003c');

export const PLAYER_CSP = "sandbox allow-scripts; default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; img-src data:";

const STAGE_CSS = `#stage { position: absolute; left: 0; top: 0; width: 1920px; height: 1080px; transform-origin: 0 0; overflow: hidden;
  --ground:#0f1020; --main:#a08cff; --second:#e8e9f5; --accent:#4fd8cf;
  --sans:"IBM Plex Sans", system-ui, sans-serif; --mono:"JetBrains Mono", ui-monospace, monospace;
  color: var(--second); font-family: var(--sans); line-height: normal; }
#stage .p-scene { position: absolute; inset: 0; }
#stage p, #stage h1, #stage h2, #stage pre { margin: 0; }
#stage .ground { position:absolute; inset:0; background:var(--ground) }
#stage .words { position:absolute; margin:0; display:flex; flex-direction:column; gap:24px } #stage .words > * { margin:0 }
#stage .line { font-family:var(--display, var(--sans, system-ui, sans-serif)); font-size:88px; line-height:1.05; font-weight:700; letter-spacing:-0.02em }
#stage .sub { font-size:36px; line-height:1.35; color:color-mix(in srgb, var(--second) 72%, var(--ground)) }
#stage .mono { font-family:var(--mono, ui-monospace, monospace) }
#stage .head { font-family:var(--display, var(--sans, system-ui, sans-serif)); font-size:60px; line-height:1.1; font-weight:700; letter-spacing:-0.015em }
#stage .eyebrow { font-family:var(--mono, ui-monospace, monospace); font-size:26px; letter-spacing:.14em; text-transform:uppercase; color:var(--accent) }
#stage .art { position:absolute; inset:0; width:100%; height:100%; overflow:visible }
#stage .node { position:absolute; box-sizing:border-box; border-radius:22px; border:3px solid color-mix(in srgb, var(--second) 28%, var(--ground)); background:color-mix(in srgb, var(--ground) 90%, var(--second)); display:flex; flex-direction:column; justify-content:center; gap:10px; padding:0 32px }
#stage .node-main { border-color:var(--main); background:color-mix(in srgb, var(--main) 16%, var(--ground)) }
#stage .node-fast { border-color:var(--accent); background:color-mix(in srgb, var(--accent) 12%, var(--ground)) }
#stage .node-strong { border-color:color-mix(in srgb, var(--main) 55%, var(--second)); background:color-mix(in srgb, var(--main) 26%, var(--ground)) }
#stage .node-dash { border-style:dashed; background:transparent } #stage .node > * { margin:0 }
#stage .node-title { font-size:38px; font-weight:700; line-height:1.1 }
#stage .node-note { font-size:27px; line-height:1.3; color:color-mix(in srgb, var(--second) 66%, var(--ground)) }
#stage .wire { fill:none; stroke:color-mix(in srgb, var(--second) 24%, var(--ground)); stroke-width:4; stroke-linecap:round }
#stage .wire-lit { fill:none; stroke:var(--accent); stroke-width:6; stroke-linecap:round }
#stage .pulse { fill:var(--accent) }
#stage .tag { font-family:var(--mono, ui-monospace, monospace); font-size:28px; color:var(--accent) }
#stage .sl-root { position:absolute; inset:0; z-index:5; background:var(--ground); opacity:0; transition:opacity .35s ease-out }
#stage .sl-root.on { opacity:1 }
#stage .sl-root .words > * { opacity:0; transform:translateX(26px); transition:opacity .5s ease-out, transform .5s ease-out }
#stage .sl-root.on .words > * { opacity:1; transform:none }
#stage .sl-code { position:absolute; left:1040px; top:250px; width:760px; max-height:700px; overflow:hidden; box-sizing:border-box; margin:0; padding:36px 40px; border-radius:22px;
  background:color-mix(in srgb, var(--ground) 88%, var(--main)); font-family:var(--mono, ui-monospace, monospace); font-size:26px; line-height:1.55; white-space:pre-wrap; color:var(--second);
  opacity:0; transform:translateY(20px); transition:opacity .5s ease-out .4s, transform .5s ease-out .4s }
#stage .sl-root.on .sl-code { opacity:1; transform:none }`;

const KIT = `const K = {
  clamp01: (x) => Math.min(1, Math.max(0, x)),
  seg: (t, a, b) => K.clamp01((t - a) / (b - a)),
  lerp: (a, b, p) => a + (b - a) * p,
  eo: (x) => 1 - Math.pow(1 - x, 3),
  ei: (x) => x * x * x,
  eio: (x) => (x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2),
  eob: (x, s = 1.4) => 1 + (s + 1) * Math.pow(x - 1, 3) + s * Math.pow(x - 1, 2),
  breathe: (t, amt = 0.012, period = 3.2) => 1 + amt * Math.sin((2 * Math.PI * t) / period),
  rng(seed) { let a = seed >>> 0; return () => { a = (a + 0x6d2b79f5) >>> 0; let r = Math.imul(a ^ (a >>> 15), 1 | a); r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r; return ((r ^ (r >>> 14)) >>> 0) / 4294967296; }; },
  h(html) { const d = document.createElement('div'); d.innerHTML = html.trim(); return d.firstElementChild; },
  typist(text, t0, cps, seed = 7) {
    const r = K.rng(seed), times = [];
    let x = t0;
    for (let i = 0; i < text.length; i++) { x += (0.55 + 0.9 * r()) / cps; times.push(x); }
    const count = (t) => { let n = 0; while (n < times.length && times[n] <= t) n++; return n; };
    return { times, end: times.length ? times[times.length - 1] : t0, count, at: (t) => text.slice(0, count(t)) };
  },
  TextFx: {
    fadeFromRight(el, t, t0, o = {}) {
      const st = o.stagger ?? 0.11, du = o.dur ?? 0.5, dx = o.dx ?? 26;
      if (!el.__w) {
        const words = el.textContent.split(' ');
        el.textContent = '';
        el.__w = words.map((w, i) => {
          if (i) el.appendChild(document.createTextNode(' '));
          const s = document.createElement('span');
          s.textContent = w; s.style.display = 'inline-block';
          el.appendChild(s);
          return s.style;
        });
      }
      el.__w.forEach((s, i) => { const p = K.eo(K.seg(t, t0 + i * st, t0 + i * st + du)); s.opacity = p; s.transform = 'translateX(' + dx * (1 - p) + 'px)'; });
      return t0 + (el.__w.length - 1) * st + du;
    },
  },
};`;

const PLAYER = `
const DEFS = {};
const SCENES = [];
const stage = document.getElementById('stage');
const boxes = {};

function boot() {
  SCENES.forEach((mod) => {
    const box = document.createElement('div');
    box.className = 'p-scene';
    box.hidden = true;
    stage.appendChild(box);
    const kit = Object.create(K);
    kit.stage = box;
    kit.scene = (id, def) => { DEFS[id] = def; boxes[id] = box; };
    try {
      mod(kit);
      const id = Object.keys(DEFS).find((k) => boxes[k] === box);
      if (id) DEFS[id].init(kit);
    } catch (e) { console.error('scene', e); }
  });
  VIDEO.scenes.forEach((s) => {
    s.pts = [[0, 0]];
    (s.lines || []).forEach((l, k) => { const a = (s.anchors || [])[k]; if (a != null && a < s.visual) s.pts.push([l.t0 - s.start, a]); });
    s.pts.push([s.dur, s.visual]);
  });
}

function warp(s, x) {
  const p = s.pts;
  if (x <= 0) return x;
  for (let i = 1; i < p.length; i++) if (x <= p[i][0]) return K.lerp(p[i - 1][1], p[i][1], (x - p[i - 1][0]) / (p[i][0] - p[i - 1][0] || 1));
  return s.visual + (x - p[p.length - 1][0]);
}

let shown = null;
function frame(t) {
  const list = VIDEO.scenes;
  let s = list[0];
  for (const x of list) if (t >= x.start) s = x;
  if (!s) return;
  if (shown !== s.id) { Object.entries(boxes).forEach(([id, b]) => { b.hidden = id !== s.id; }); shown = s.id; }
  try { DEFS[s.id] && DEFS[s.id].render(warp(s, t - s.start)); } catch (e) { console.error('render', s.id, e); }
}

const slideRoot = document.createElement('div');
slideRoot.className = 'sl-root';
slideRoot.hidden = true;
stage.appendChild(slideRoot);
function slide(sl) {
  if (!sl) { slideRoot.classList.remove('on'); setTimeout(() => { if (!slideRoot.classList.contains('on')) slideRoot.hidden = true; }, 400); return; }
  slideRoot.textContent = '';
  const words = document.createElement('div');
  words.className = 'words';
  const code = typeof sl.code === 'string' && sl.code.trim() ? sl.code : null;
  words.style.cssText = 'left:120px;top:110px;width:' + (code ? 860 : 1680) + 'px';
  const eye = document.createElement('p'); eye.className = 'eyebrow'; eye.textContent = sl.eyebrow || 'Answer';
  const h = document.createElement('h2'); h.className = 'head'; h.textContent = sl.title || '';
  words.append(eye, h);
  (sl.points || []).forEach((pt, i) => {
    const p = document.createElement('p'); p.className = 'sub'; p.textContent = pt;
    p.style.transitionDelay = (0.25 + i * 0.35) + 's';
    words.appendChild(p);
  });
  slideRoot.appendChild(words);
  if (code) { const pre = document.createElement('pre'); pre.className = 'sl-code'; pre.textContent = code; slideRoot.appendChild(pre); }
  slideRoot.hidden = false;
  // A moment after it is shown, so its entrance transitions run (rAF can be throttled in a frame).
  setTimeout(() => slideRoot.classList.add('on'), 40);
}

function fit() {
  const z = Math.min(innerWidth / 1920, innerHeight / 1080);
  stage.style.transform = 'translate(' + (innerWidth - 1920 * z) / 2 + 'px,' + (innerHeight - 1080 * z) / 2 + 'px) scale(' + z + ')';
}
addEventListener('resize', fit);
addEventListener('message', (e) => {
  const m = e.data || {};
  if (m.type === 'frame' && typeof m.t === 'number') frame(m.t);
  else if (m.type === 'slide') slide(m.slide || null);
});

// After scenes.js (the next script) has pushed its modules.
function start() {
  boot();
  fit();
  frame(0);
  parent.postMessage({ type: 'ready' }, '*');
}
`;

/** The player page for `video` (videos.mjs's shape) and its scenes' code. */
export function playerHtml({ video, scenesCode }) {
  const clean = { scenes: video.scenes.map((s) => ({ id: s.id, start: s.start, dur: s.dur, visual: s.visual ?? s.dur, anchors: s.anchors ?? null, lines: s.lines ?? [] })) };
  return `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;700&family=JetBrains+Mono:wght@400;700&display=swap">
<style>
html, body { margin:0; height:100%; overflow:hidden; background:#0f1020 }
[hidden] { display:none !important }
${STAGE_CSS}
</style></head>
<body><div id="stage"></div>
<script>
const VIDEO = ${inline(clean)};
${KIT}
${PLAYER}
</script>
<script>
${String(scenesCode ?? '').replace(/<\/script/gi, '<\\/script')}
</script>
<script>start();</script>
</body></html>`;
}
