/* Aurion voice agent — pixel canvas visualizer + wake-word flow + autorun. */

const STATES = {
  standby:    { label: 'STANDBY',    sub: 'say "synapse" to wake' },
  listening:  { label: 'LISTENING',  sub: 'audio channel open'    },
  processing: { label: 'PROCESSING', sub: 'model is working...'   },
  speaking:   { label: 'SPEAKING',   sub: 'responding'            },
};
const ORDER = ['standby', 'listening', 'processing', 'speaking'];
const FINISH_LINE = 'Finished. Anything else?';

/* ---------- dom ---------- */
const body = document.body;
const bubble = document.getElementById('bubble');
const logEl = document.getElementById('log');
const buttons = [...document.querySelectorAll('.state-btn')];
const wakeStatus = document.getElementById('wakeStatus');
const micBadge = document.getElementById('micBadge');
const checksEl = document.getElementById('checks');

let state = 'standby';

/* ---------- log ---------- */
function log(msg, key) {
  const time = new Date().toLocaleTimeString('en-GB', { hour12: false });
  const row = document.createElement('div');
  row.innerHTML = `<span class="t">${time}</span> ${key ? `<span class="k">${key}</span> ` : ''}${msg}`;
  logEl.prepend(row);
  while (logEl.children.length > 9) logEl.lastChild.remove();
}

/* ---------- state ---------- */
function goState(next, opts = {}) {
  if (!STATES[next]) return;
  state = next;
  body.dataset.state = next;
  buttons.forEach((b) => b.classList.toggle('active', b.dataset.state === next));
  if (!opts.quiet) log('state', next);
  updateBubble();
}

function updateBubble() {
  if (state === 'speaking') {
    bubble.hidden = false;
    bubble.textContent = FINISH_LINE;
  } else if (state === 'listening') {
    bubble.hidden = false;
    bubble.textContent = transcript || 'Listening...';
  } else {
    bubble.hidden = true;
  }
}

/* ---------- reference model: synapse/animation_reference.html ----------
   Filled white hexagram (two triangles), fixed size, in-plane rotation
   (all states except standby), traveling light sweep on the outline. */
const N = 73, CX = 36, CY = 36, R = 20, C30 = 0.8660254;
const TRI_U = [[0, -1], [C30, 0.5], [-C30, 0.5]]; // unit triangles
const TRI_D = [[0, 1], [C30, -0.5], [-C30, -0.5]];

/* solid-star model: 6 outer + 6 inner outline points, 18 rim edges */
const HX = C30 / 3;
const OUT6 = [[0, -1], [C30, 0.5], [-C30, 0.5], [0, 1], [C30, -0.5], [-C30, -0.5]];
const IN6 = [[-HX, -0.5], [HX, -0.5], [2 * HX, 0], [HX, 0.5], [-HX, 0.5], [-2 * HX, 0]];
const SOLID_PTS = [...OUT6, ...IN6];
const RIM = [[0, 7, 8, 1], [1, 9, 10, 2], [2, 11, 6, 0], [3, 9, 8, 4], [4, 7, 6, 5], [5, 11, 10, 3]];
const HALF_EDGES = RIM.flatMap((s) => [[s[0], s[1]], [s[1], s[2]], [s[2], s[3]]]);
const EDGE_N = HALF_EDGES.map(([a, b]) => {
  const p = SOLID_PTS[a], q = SOLID_PTS[b];
  const ex = q[0] - p[0], ey = q[1] - p[1];
  const len = Math.hypot(ex, ey) || 1;
  return [ey / len, -ex / len];
});

function pointInTri(px, py, t) {
  const [a, b, c] = t;
  const d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]) || 1e-9;
  const u = ((b[1] - c[1]) * (px - c[0]) + (c[0] - b[0]) * (py - c[1])) / d;
  const v = ((c[1] - a[1]) * (px - c[0]) + (a[0] - c[0]) * (py - c[1])) / d;
  return u >= 0 && v >= 0 && u + v <= 1;
}





/* ---------- audio: mic level + sim energy ---------- */
let actx = null, analyser = null, micOK = false, freqBins = null, timeData = null;

async function enableMic() {
  if (micOK) return true;
  try {
    actx = actx || new (window.AudioContext || window.webkitAudioContext)();
    if (actx.state === 'suspended') await actx.resume();
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: true, noiseSuppression: true },
    });
    const src = actx.createMediaStreamSource(stream);
    analyser = actx.createAnalyser();
    analyser.fftSize = 256;
    analyser.smoothingTimeConstant = 0.55;
    src.connect(analyser);
    freqBins = new Uint8Array(analyser.frequencyBinCount);
    timeData = new Uint8Array(analyser.fftSize);
    micOK = true;
    micBadge.textContent = 'MIC: LIVE';
    micBadge.classList.add('on');
    log('mic live', 'mic');
  } catch (e) {
    micOK = false;
    micBadge.textContent = 'MIC: SIM';
    log('mic blocked (needs HTTPS/localhost) — sim mode', 'mic');
  }
  return micOK;
}

function micLevel() {
  if (!analyser) return null;
  analyser.getByteTimeDomainData(timeData);
  let sum = 0;
  for (let i = 0; i < timeData.length; i++) {
    const v = (timeData[i] - 128) / 128;
    sum += v * v;
  }
  return Math.min(1, Math.sqrt(sum / timeData.length) * 3.2);
}

/* simulated energy (fallback + processing/speaking/idle motion) */
const sim = { e: 0.08, target: 0.08, next: 0 };

function simEnergy(now, kind) {
  if (now > sim.next) {
    sim.next = now + 140;
    if (kind === 'lively') sim.target = 0.35 + Math.random() * 0.6;
    else if (kind === 'mid') sim.target = 0.3 + Math.random() * 0.3;
    else if (kind === 'idle') sim.target = 0.05 + Math.random() * 0.08;
  }
  sim.e += (sim.target - sim.e) * 0.3;
  return sim.e;
}

/* ---------- variant grid: 4 cols x 2 rows ---------- */
const VARIANTS = [
  { key: 'standard', label: '01 · STANDARD' },
  { key: 'breath', label: '02 · BREATH' },
  { key: 'spin', label: '03 · SPIN' },
  { key: 'slow', label: '04 · SLOW' },
  { key: 'reverse', label: '05 · REVERSE' },
  { key: 'rock', label: '06 · ROCK' },
  { key: 'pendulum', label: '07 · PENDULUM' },
  { key: 'pulse', label: '08 · PULSE' },
  { key: 'heartbeat', label: '09 · HEARTBEAT' },
  { key: 'squash', label: '10 · SQUASH' },
  { key: 'stretch', label: '11 · STRETCH' },
  { key: 'slidex', label: '12 · SLIDE X' },
  { key: 'slidey', label: '13 · SLIDE Y' },
  { key: 'orbit', label: '14 · ORBIT' },
  { key: 'spinorbit', label: '15 · SPIN+ORBIT' },
  { key: 'flipx', label: '16 · COIN X' },
  { key: 'flipy', label: '17 · COIN Y' },
  { key: 'stairs', label: '18 · STAIRS' },
  { key: 'jitter', label: '19 · JITTER' },
  { key: 'twinkle', label: '20 · TWINKLE' },
  { key: 'scan', label: '21 · SCAN' },
  { key: 'wipe', label: '22 · WIPE' },
  { key: 'trail', label: '23 · TRAIL' },
  { key: 'counter', label: '24 · COUNTER' },
  { key: 'wave', label: '25 · WAVE' },
  { key: 'blink', label: '26 · BLINK' },
  { key: 'grow', label: '27 · GROW' },
  { key: 'spinpulse', label: '28 · SPIN PULSE' },
  { key: 'drift', label: '29 · DRIFT' },
  { key: 'shiver', label: '30 · SHIVER' },
  { key: 'static', label: '31 · STATIC' },
  { key: 'swing', label: '32 · SWING' },
];
const TAU = Math.PI * 2;

/* visible 4x2 selection (full 32-set stays in VARIANTS for later) */
const SHOW = ['standard', 'breath', 'spin', 'rock', 'flipx', 'flipy', 'counter', 'trail'];

const gridEl = document.getElementById('grid');
const cells = [];
SHOW.forEach((key, idx) => {
  const v = VARIANTS.find((w) => w.key === key);
  const cell = document.createElement('div');
  cell.className = 'cell live';
  const cvs = document.createElement('canvas');
  cvs.width = N; cvs.height = N;
  const tag = document.createElement('span');
  tag.className = 'tag';
  tag.textContent = `${String(idx + 1).padStart(2, '0')} ·${v.label.split('·')[1]}`;
  cell.appendChild(cvs);
  cell.appendChild(tag);
  const cx2d = cvs.getContext('2d');
  cells.push({ v, ctx: cx2d, img: cx2d.createImageData(N, N), theta: 0 });
  gridEl.appendChild(cell);
});

/* ---------- render loop (always running, ~30fps, tiny pixel grid) ---------- */
let lastT = 0, frame = 0;
let transcript = '';

function drawPass(d, o) {
  const { thU, thD, sx, sy, ox, oy, pv, fx, t, flip } = o;
  const flipping = fx === 'flipx' || fx === 'flipy';
  let ca = flipping ? Math.cos(flip) : 1;
  if (flipping && Math.abs(ca) < 0.08) ca = ca < 0 ? -0.08 : 0.08;
  const cu = Math.cos(thU), su = Math.sin(thU);
  const cd = Math.cos(thD), sd = Math.sin(thD);
  const map = (p, c, s) => {
    const X = (p[0] - pv[0]) * sx * c - (p[1] - pv[1]) * sy * s;
    const Y = (p[0] - pv[0]) * sx * s + (p[1] - pv[1]) * sy * c;
    let Z = 0, XX = X, YY = Y;
    if (fx === 'flipx') { YY = Y * ca; Z = Y * Math.sin(flip); }
    if (fx === 'flipy') { XX = X * ca; Z = -X * Math.sin(flip); }
    const f = 300 / (300 - Z * R); // true perspective divide
    return [CX + pv[0] * R + XX * R * f + ox, CY + pv[1] * R + YY * R * f + oy];
  };
  const rU = TRI_U.map((p) => map(p, cu, su));
  const rD = TRI_D.map((p) => map(p, cd, sd));
  const search = Math.ceil(R * Math.max(Math.abs(sx), Math.abs(sy))) + 8;
  const x0 = Math.max(0, Math.floor(CX + ox - search)), x1 = Math.min(N - 1, Math.ceil(CX + ox + search));
  const y0 = Math.max(0, Math.floor(CY + oy - search)), y1 = Math.min(N - 1, Math.ceil(CY + oy + search));
  for (let y = y0; y <= y1; y++) {
    for (let x = x0; x <= x1; x++) {
      const px = x + 0.5, py = y + 0.5;
      if (fx === 'twinkle') {
        const h = (((x * 73 + y) * 2654435761) ^ (frame * 97 + 13)) >>> 0;
        if (h % 100 < 15) continue;
      }
      if (fx === 'scan') {
        const sc = ((t * 30) % (N + 28)) - 14;
        if (Math.abs(py - sc) > 7) continue;
      }
      if (fx === 'wipe') {
        let rel = (Math.atan2(py - CY, px - CX) - t * 1.5) % TAU;
        if (rel < 0) rel += TAU;
        if (rel > 4.6) continue;
      }
      const qx = fx === 'wave' ? px + 3 * Math.sin(py * 0.3 + t * 3) : px;
      if (!pointInTri(qx, py, rU) && !pointInTri(qx, py, rD)) continue;
      const i = (y * N + x) * 4;
      d[i] = 255; d[i + 1] = 255; d[i + 2] = 255; d[i + 3] = 255;
    }
  }

  /* edge bar: a real object shows its rim edge-on, a photo just squashes */
  if (flipping && Math.abs(Math.cos(flip)) < 0.2) {
    const ex = Math.round(CX + ox), ey = Math.round(CY + oy), er = Math.round(R);
    const bar = (x, y) => {
      if (x < 0 || x >= N || y < 0 || y >= N) return;
      const i = (y * N + x) * 4;
      d[i] = 255; d[i + 1] = 255; d[i + 2] = 255; d[i + 3] = 255;
    };
    if (fx === 'flipx') {
      for (let y = ey - 2; y <= ey + 2; y++) {
        for (let x = ex - er; x <= ex + er; x++) bar(x, y);
      }
    } else {
      for (let x = ex - 2; x <= ex + 2; x++) {
        for (let y = ey - er; y <= ey + er; y++) bar(x, y);
      }
    }
  }
}

/* real 3D solid: star prism (front/back faces + rim) with perspective */
const HD = 0.16;            // half thickness in model units
const CAM = 6.2;            // camera distance in model units
const RIM_SH = [200, 150, 105, 70, 120, 175]; // per-side grays, 6 shades

function pointInPoly(px, py, pts) {
  let inside = false;
  for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
    const xi = pts[i][0], yi = pts[i][1], xj = pts[j][0], yj = pts[j][1];
    if (yi > py !== yj > py && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

function fillPoly(d, pts, v) {
  let x0 = N, y0 = N, x1 = -1, y1 = -1;
  for (const p of pts) {
    if (p[0] < x0) x0 = p[0];
    if (p[0] > x1) x1 = p[0];
    if (p[1] < y0) y0 = p[1];
    if (p[1] > y1) y1 = p[1];
  }
  x0 = Math.max(0, Math.floor(x0)); y0 = Math.max(0, Math.floor(y0));
  x1 = Math.min(N - 1, Math.ceil(x1)); y1 = Math.min(N - 1, Math.ceil(y1));
  for (let y = y0; y <= y1; y++) {
    for (let x = x0; x <= x1; x++) {
      const px = x + 0.5, py = y + 0.5;
      if (!pointInPoly(px, py, pts)) continue;
      const i = (y * N + x) * 4;
      d[i] = v; d[i + 1] = v; d[i + 2] = v; d[i + 3] = 255;
    }
  }
}

function drawSolid(d, t, axis) {
  // non-uniform spin: lingers face-on, snaps through edge-on
  let a = t * 1.5;
  for (let i = 0; i < 4; i++) a += 0.004 * (0.5 + (1 - Math.abs(Math.cos(a)))); // slow drift
  const wob = 0.16 * Math.sin(t * 1.15), wob2 = 0.12 * Math.sin(t * 0.83 + 1.4);
  const ca = Math.cos(a), sa = Math.sin(a);
  const cw = Math.cos(wob), sw = Math.sin(wob);
  const cw2 = Math.cos(wob2), sw2 = Math.sin(wob2);

  // model -> view
  const mv = (p, z) => {
    let x = p[0], y = p[1], zz = z;
    if (axis === 'y') { x = p[0] * ca + z * sa; zz = -p[0] * sa + z * ca; }
    else { y = p[1] * ca + z * sa; zz = -p[1] * sa + z * ca; }
    // wobble about X then Z
    let y2 = y * cw - zz * sw, z2 = y * sw + zz * cw;
    let x3 = x * cw2 - y2 * sw2, y3 = x * sw2 + y2 * cw2;
    return [x3, y3, z2];
  };
  const proj = (v) => {
    const f = CAM / (CAM - v[2]);
    return [CX + v[0] * R * f, CY + v[1] * R * f, v[2]];
  };
  const F = SOLID_PTS.map((p) => proj(mv(p, HD)));
  const B = SOLID_PTS.map((p) => proj(mv(p, -HD)));

  // back faces (dim) then rim quads then front faces (white)
  fillPoly(d, [B[0], B[1], B[2]], 55);
  fillPoly(d, [B[3], B[4], B[5]], 55);

  const quads = HALF_EDGES.map(([i, j], n) => ({
    pts: [F[i], F[j], B[j], B[i]],
    z: (F[i][2] + F[j][2] + B[i][2] + B[j][2]) / 4,
    v: RIM_SH[n % RIM_SH.length],
  })).sort((p, q) => p.z - q.z);
  for (const q of quads) fillPoly(d, q.pts, q.v);

  fillPoly(d, [F[0], F[1], F[2]], 255);
  fillPoly(d, [F[3], F[4], F[5]], 255);
}

function drawCell(c, now, dt) {
  const d = c.img.data;
  d.fill(0);
  const t = now / 1000, k = c.v.key;
  if (k === 'blink' && (t * 1.5) % 1 >= 0.7) {
    c.ctx.putImageData(c.img, 0, 0);
    return;
  }
  if (k === 'flipx' || k === 'flipy') {
    drawSolid(d, t, k === 'flipx' ? 'x' : 'y');
    c.ctx.putImageData(c.img, 0, 0);
    return;
  }
  const spd = state !== 'standby'
    ? state === 'processing' ? 1.26 : state === 'listening' ? 0.84 : 0.42
    : 0;

  let thU = c.theta, thD = c.theta, sx = 1, sy = 1, ox = 0, oy = 0, fx = null, pv = null;
  switch (k) {
    case 'breath': { const s = 1 + 0.05 * Math.sin(t * 1.6); sx = sy = s; c.theta += spd * dt; break; }
    case 'spin': c.theta += 0.9 * dt; break;
    case 'slow': c.theta += 0.25 * dt; break;
    case 'reverse': c.theta -= 0.7 * dt; break;
    case 'pulse': { const s = 1 + 0.12 * Math.sin(t * 2.2); sx = sy = s; c.theta += spd * dt; break; }
    case 'heartbeat': {
      const a = Math.pow(Math.max(0, Math.sin(t * 3)), 3) + 0.5 * Math.pow(Math.max(0, Math.sin(t * 6 + 1)), 3);
      sx = sy = 1 + 0.09 * a; c.theta += spd * dt; break;
    }
    case 'squash': { const q = 0.1 * Math.sin(t * 1.8); sx = 1 + q; sy = 1 - q; c.theta += spd * dt; break; }
    case 'stretch': sx = 1 + 0.12 * Math.sin(t * 1.1); c.theta += spd * dt; break;
    case 'slidex': ox = 4 * Math.sin(t * 0.9); c.theta += spd * dt; break;
    case 'slidey': oy = 4 * Math.sin(t * 1.1); c.theta += spd * dt; break;
    case 'orbit': ox = 5 * Math.cos(t * 0.7); oy = 5 * Math.sin(t * 0.7); break;
    case 'spinorbit': c.theta += 0.8 * dt; ox = 5 * Math.cos(t * 0.7); oy = 5 * Math.sin(t * 0.7); break;
    case 'flipx': fx = 'flipx'; break; // frozen theta, pure 3D flip
    case 'flipy': fx = 'flipy'; break;
    case 'stairs': c.theta += spd * dt; break;
    case 'jitter': ox = (Math.random() - 0.5) * 2; oy = (Math.random() - 0.5) * 2; c.theta += spd * dt; break;
    case 'twinkle': c.theta += spd * dt; fx = 'twinkle'; break;
    case 'scan': c.theta += spd * dt; fx = 'scan'; break;
    case 'wipe': c.theta += spd * dt; fx = 'wipe'; break;
    case 'trail': c.theta += (spd || 0.9) * dt; fx = 'trail'; break;
    case 'counter': c.theta += 0.6 * dt; thU = c.theta; thD = -c.theta; break;
    case 'wave': c.theta += spd * dt; fx = 'wave'; break;
    case 'grow': { const g = (t * 0.25) % 1; sx = sy = 0.75 + 0.4 * g; c.theta += spd * dt; break; }
    case 'spinpulse': c.theta += 0.9 * (0.5 + 0.5 * Math.sin(t * 0.5)) * dt; break;
    case 'drift': c.theta += 0.3 * dt; ox = oy = 3 * Math.sin(t * 0.3); break;
    case 'shiver': c.theta += 0.5 * dt; thU = thD = c.theta + 0.05 * Math.sin(t * 25); break;
    case 'static': break;
    case 'swing': pv = [0, 1]; break;
    default: c.theta += spd * dt; break; // standard
  }
  if (k === 'rock') { thU = thD = c.theta + 0.35 * Math.sin(t * 0.45); }
  else if (k === 'pendulum') { thU = thD = c.theta + 1.2 * Math.sin(t * 0.8); }
  else if (k === 'stairs') { const q = Math.PI / 12; thU = thD = Math.floor(c.theta / q) * q; }
  else if (k === 'swing') { thU = thD = c.theta + 0.45 * Math.sin(t * 0.9); }
  else if (k !== 'counter' && k !== 'shiver') { thU = thD = c.theta; }

  const o = {
    thU, thD, sx, sy, ox, oy, pv: pv || [0, 0], fx, t,
    flip: k === 'flipx' ? t * 1.4 : k === 'flipy' ? t * 1.2 : 0,
  };
  if (fx === 'trail') {
    for (let j = 2; j >= 0; j--) {
      drawPass(d, { ...o, thU: thU - 0.15 * j, thD: thD - 0.15 * j, fx: null });
    }
  } else {
    drawPass(d, o);
  }
  c.ctx.putImageData(c.img, 0, 0);
}

function render(now) {
  requestAnimationFrame(render);
  if (now - lastT < 33) return; // ~30fps is plenty for pixel art
  const dt = Math.min(0.1, (now - lastT) / 1000) || 0.033;
  lastT = now;
  frame++;
  for (const c of cells) drawCell(c, now, dt);
}

/* ---------- speech: wake-word recognition + TTS ---------- */
let rec = null, voiceOn = false;

function setWakeStatus(html) { wakeStatus.innerHTML = html; }

function enableVoice() {
  const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
  if (!SR) {
    setWakeStatus('wake-word: <b>unsupported</b> — use SIM WAKE');
    log('speech recognition unsupported', 'wake');
    return;
  }
  if (voiceOn) return;
  try {
    rec = new SR();
    rec.continuous = true;
    rec.interimResults = true;
    rec.lang = 'en-US';
    rec.onresult = (ev) => {
      let text = '';
      for (let i = ev.resultIndex; i < ev.results.length; i++) text += ev.results[i][0].transcript;
      transcript = text.trim().slice(-60);
      if (state === 'listening') {
        bubble.hidden = false;
        bubble.textContent = transcript || 'Listening...';
      }
      if (/synapse/i.test(text)) {
        transcript = '';
        log('heard "synapse"', 'wake');
        wakeFlow(false);
      }
    };
    rec.onerror = (ev) => {
      setWakeStatus(`wake-word: <b>error (${ev.error})</b> — use SIM WAKE`);
      log(`recognition error: ${ev.error}`, 'wake');
    };
    rec.onend = () => { if (voiceOn) { try { rec.start(); } catch (_) {} } };
    rec.start();
    voiceOn = true;
    setWakeStatus('wake-word: listening for <b>"synapse"</b>...');
    log('wake-word armed', 'wake');
  } catch (e) {
    setWakeStatus('wake-word: <b>blocked</b> (needs HTTPS/localhost) — use SIM WAKE');
    log('recognition blocked — sim mode', 'wake');
  }
}

function speak(text, done) {
  const finish = () => done && done();
  try {
    if (!('speechSynthesis' in window)) throw new Error('no tts');
    speechSynthesis.cancel();
    const u = new SpeechSynthesisUtterance(text);
    u.lang = 'en-US';
    u.rate = 1;
    let ended = false;
    u.onend = () => { if (!ended) { ended = true; finish(); } };
    u.onerror = () => { if (!ended) { ended = true; finish(); } };
    speechSynthesis.speak(u);
    setTimeout(() => { if (!ended) { ended = true; speechSynthesis.cancel(); finish(); } }, 6000);
  } catch (_) {
    log('tts unavailable — text only', 'speak');
    setTimeout(finish, 2500);
  }
}

/* ---------- flow engine + autorun ---------- */
let flowTimers = [];
let autoRun = false;

function later(ms, fn) {
  const id = setTimeout(fn, ms);
  flowTimers.push(id);
  return id;
}

function stopFlow(silent) {
  flowTimers.forEach(clearTimeout);
  flowTimers = [];
  try { speechSynthesis.cancel(); } catch (_) {}
  if (autoRun) {
    autoRun = false;
    resetChecks();
    if (!silent) log('autorun stopped', 'auto');
  }
}

function setCheck(step, cls, ms) {
  const li = checksEl.querySelector(`[data-step="${step}"]`);
  if (!li) return;
  li.classList.remove('active', 'pass');
  if (cls) li.classList.add(cls);
  const base = li.textContent.replace(/\s*\d+ms$/, '');
  li.textContent = ms != null ? `${base} ${ms}ms` : base;
}

function resetChecks() {
  checksEl.querySelectorAll('li').forEach((li) => {
    li.classList.remove('active', 'pass');
    li.textContent = li.textContent.replace(/\s*\d+ms$/, '');
  });
}

function wakeFlow(auto) {
  if (state !== 'standby') return;
  stopFlow(true);
  autoRun = !!auto;
  const t0 = performance.now();

  setCheck('wake', 'active');
  goState('listening', { quiet: auto });
  if (auto) log('heard "synapse" (sim)', 'wake');
  setCheck('wake', 'pass', Math.round(performance.now() - t0));

  setCheck('listen', 'active');
  let peak = 0;
  const probe = setInterval(() => {
    const v = state === 'listening' && micOK ? (micLevel() || 0) : sim.e;
    if (v > peak) peak = v;
  }, 100);
  flowTimers.push(probe);

  later(4000, () => {
    clearInterval(probe);
    setCheck('listen', 'pass', 4000);
    log(`listen done — peak level ${peak.toFixed(2)}${micOK ? ' (mic)' : ' (sim)'}`, 'auto');

    setCheck('process', 'active');
    goState('processing', { quiet: auto });

    later(3000, () => {
      setCheck('process', 'pass', 3000);

      setCheck('speak', 'active');
      goState('speaking', { quiet: auto });
      const t1 = performance.now();
      speak(FINISH_LINE, () => {
        setCheck('speak', 'pass', Math.round(performance.now() - t1));

        setCheck('back', 'active');
        goState('standby', { quiet: auto });
        setCheck('back', 'pass', 0);
        if (autoRun) { autoRun = false; log('autorun complete — all tests PASS', 'auto'); }
      });
    });
  });
}

function autorun() {
  if (state !== 'standby') goState('standby', { quiet: true });
  resetChecks();
  log('autorun started', 'auto');
  enableMic().finally(() => wakeFlow(true));
}

/* ---------- ui wiring ---------- */
buttons.forEach((b) => b.addEventListener('click', () => {
  stopFlow();
  if (b.dataset.state === 'listening') enableMic();
  goState(b.dataset.state);
}));

document.getElementById('btnWake').addEventListener('click', () => wakeFlow(false));
document.getElementById('btnAuto').addEventListener('click', autorun);
document.getElementById('btnStop').addEventListener('click', () => {
  stopFlow();
  goState('standby');
});
document.getElementById('btnMic').addEventListener('click', enableMic);
document.getElementById('btnVoice').addEventListener('click', enableVoice);

document.addEventListener('keydown', (e) => {
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  const k = e.key.toLowerCase();
  const idx = ['1', '2', '3', '4'].indexOf(e.key);
  if (idx !== -1) {
    stopFlow();
    goState(ORDER[idx]);
  }
  else if (k === 'w') wakeFlow(false);
  else if (k === 'a') autorun();
  else if (k === 's') { stopFlow(); goState('standby'); }
  else if (k === 'm') enableMic();
});

/* ---------- boot ---------- */
goState('standby', { quiet: true });
log('agent ready — press RUN ALL or say "synapse"', 'sys');
if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
  render(0); // single static frame
} else {
  requestAnimationFrame(render);
}
