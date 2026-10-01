// God's Eye View renderer.
//
// Loads the WASM engine, advances it on a fixed 60 Hz accumulator, and draws
// the 22 players + ball directly from the engine's flat render buffer using a
// zero-copy Float32Array view into WASM linear memory.

import init, { WasmMatchEngine } from './pkg/match_engine.js';

const ENTITY_STRIDE = 6;
const PITCH_W_M = 105.0;
const PITCH_H_M = 68.0;
const FIXED_DT = 1 / 60;

// Metrics-tuned pitch drawing constants (metres).
const MARGIN = 26;
const PENALTY_DEPTH = 16.5;
const PENALTY_WIDTH = 40.32;
const GOAL_AREA_DEPTH = 5.5;
const GOAL_AREA_WIDTH = 18.32;
const CENTER_CIRCLE_R = 9.15;
const PENALTY_SPOT = 11.0;

const els = {
  canvas: document.getElementById('gev-canvas'),
  timer: document.getElementById('timer'),
  fps: document.getElementById('fps'),
  entities: document.getElementById('entities'),
  altitude: document.getElementById('altitude'),
  status: document.getElementById('status'),
  playpause: document.getElementById('playpause'),
  restart: document.getElementById('restart'),
  file: document.getElementById('file'),
  speed: document.getElementById('speed'),
  speedval: document.getElementById('speedval'),
  xmlpath: document.getElementById('xmlpath'),
  loadpath: document.getElementById('loadpath'),
  feed: document.getElementById('feed'),
  splash: document.getElementById('splash'),
  score: document.getElementById('score'),
  posshome: document.getElementById('posshome'),
  possaway: document.getElementById('possaway'),
  possval: document.getElementById('possval'),
  back10: document.getElementById('back10'),
  fwd10: document.getElementById('fwd10'),
  browse: document.getElementById('browse'),
  filelist: document.getElementById('filelist'),
  matchList: document.getElementById('match-list'),
  matchInfo: document.getElementById('match-info'),
  homeName: document.getElementById('homename'),
  awayName: document.getElementById('awayname'),
};

const ctx = els.canvas.getContext('2d');

// ── World → screen ─────────────────────────────────────────────
// Engine x ∈ [-52.5, 52.5] (home attacks +x → right).
// Engine y ∈ [-34, 34] (flipped so metric +y is up on screen).
// Uniform pitch scale (metres → pixels) so the field is never stretched,
// with the pitch centred inside the canvas.
const PX_PER_M = Math.min(
  (els.canvas.width - 2 * MARGIN) / PITCH_W_M,
  (els.canvas.height - 2 * MARGIN) / PITCH_H_M,
);
const PITCH_PX_W = PITCH_W_M * PX_PER_M;
const PITCH_PX_H = PITCH_H_M * PX_PER_M;
const OFFSET_X = (els.canvas.width - PITCH_PX_W) / 2;
const OFFSET_Y = (els.canvas.height - PITCH_PX_H) / 2;
const CX = els.canvas.width / 2;
const CY = els.canvas.height / 2;

function toScreen(x, y) {
  return [
    OFFSET_X + (x + PITCH_W_M / 2) * PX_PER_M,
    OFFSET_Y + (PITCH_H_M / 2 - y) * PX_PER_M,
  ];
}

function drawPitch() {
  ctx.fillStyle = '#1e3822';
  ctx.fillRect(0, 0, els.canvas.width, els.canvas.height);

  // Mowing stripes for depth.
  ctx.fillStyle = 'rgba(255,255,255,0.03)';
  const stripeW = PITCH_PX_W / 10;
  for (let i = 0; i < 10; i += 2) {
    ctx.fillRect(OFFSET_X + i * stripeW, OFFSET_Y, stripeW, PITCH_PX_H);
  }

  ctx.strokeStyle = 'rgba(255,255,255,0.45)';
  ctx.lineWidth = 2;
  ctx.strokeRect(OFFSET_X, OFFSET_Y, PITCH_PX_W, PITCH_PX_H);

  ctx.beginPath();
  ctx.moveTo(CX, OFFSET_Y);
  ctx.lineTo(CX, OFFSET_Y + PITCH_PX_H);
  ctx.stroke();

  ctx.beginPath();
  ctx.arc(CX, CY, CENTER_CIRCLE_R * PX_PER_M, 0, Math.PI * 2);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(CX, CY, 3, 0, Math.PI * 2);
  ctx.fill();

  const paW = PENALTY_DEPTH * PX_PER_M;
  const paH = PENALTY_WIDTH * PX_PER_M;
  const paTop = (els.canvas.height - paH) / 2;
  ctx.strokeRect(OFFSET_X, paTop, paW, paH);
  ctx.strokeRect(OFFSET_X + PITCH_PX_W - paW, paTop, paW, paH);

  const gaW = GOAL_AREA_DEPTH * PX_PER_M;
  const gaH = GOAL_AREA_WIDTH * PX_PER_M;
  const gaTop = (els.canvas.height - gaH) / 2;
  ctx.strokeRect(OFFSET_X, gaTop, gaW, gaH);
  ctx.strokeRect(OFFSET_X + PITCH_PX_W - gaW, gaTop, gaW, gaH);

  const spotL = OFFSET_X + PENALTY_SPOT * PX_PER_M;
  const spotR = OFFSET_X + PITCH_PX_W - PENALTY_SPOT * PX_PER_M;
  ctx.beginPath();
  ctx.arc(spotL, CY, 2.5, 0, Math.PI * 2);
  ctx.arc(spotR, CY, 2.5, 0, Math.PI * 2);
  ctx.fill();
}

// ── State ──────────────────────────────────────────────────────
let wasm = null;
let engine = null;
let lastXml = null;
let playing = true;
let speed = 1;
let accumulator = 0;
let lastFrame = performance.now();
let fpsSmooth = 0;
let lastEventSeq = -1;
let splashTimer = null;

function setStatus(msg) {
  els.status.textContent = msg || '';
}

function destroyEngine() {
  if (engine && typeof engine.free === 'function') {
    try { engine.free(); } catch { /* ignore */ }
  }
  engine = null;
}

function startEngine(xml) {
  destroyEngine();
  lastXml = xml;
  engine = new WasmMatchEngine(xml);
  accumulator = 0;
  lastEventSeq = -1;
  els.feed.innerHTML = '';
  els.splash.classList.remove('show');
  els.restart.disabled = false;
  // Label the HUD with the match metadata straight from the engine.
  els.homeName.textContent = engine.get_home_team_name() || 'Home';
  els.awayName.textContent = engine.get_away_team_name() || 'Away';
  const meta = [engine.get_competition_id(), engine.get_season(), engine.get_game_date()]
    .filter(Boolean);
  els.matchInfo.textContent = meta.length ? meta.join(' · ') : '';
  setStatus(`${engine.get_duration_secs().toFixed(0)} s of match loaded`);
  playing = true;
  els.playpause.textContent = '⏸ Pause';
}

// ── Event feed + splash ────────────────────────
function teamName(t) {
  return t === 0 ? 'Home' : t === 1 ? 'Away' : '';
}

function showSplash(label, team) {
  const who = teamName(team);
  els.splash.innerHTML = `${label}${who ? `<span class="who">${who}</span>` : ''}`;
  els.splash.classList.add('show');
  if (splashTimer) clearTimeout(splashTimer);
  splashTimer = setTimeout(() => els.splash.classList.remove('show'), 2600);
}

function updateEvents() {
  if (!engine) return;
  const seq = engine.get_event_seq();
  if (seq === lastEventSeq) return;
  lastEventSeq = seq;

  const count = engine.get_recent_event_count();
  const rows = [];
  for (let i = count - 1; i >= Math.max(0, count - 8); i--) {
    const t = engine.get_recent_event_time(i);
    const label = engine.get_recent_event_label(i);
    const team = engine.get_recent_event_team(i);
    const notable = engine.get_recent_event_notable(i);
    const mm = String(Math.floor(t / 60)).padStart(2, '0');
    const ss = String(Math.floor(t % 60)).padStart(2, '0');
    const cls = team === 0 ? 'home' : team === 1 ? 'away' : '';
    rows.push(`<div class="row ${cls} ${notable ? 'notable' : ''}">` +
      `<span class="t">${mm}:${ss}</span><span class="lbl">${label}</span></div>`);
  }
  els.feed.innerHTML = rows.join('');

  if (count > 0) {
    const i = count - 1;
    if (engine.get_recent_event_notable(i)) {
      showSplash(engine.get_recent_event_label(i), engine.get_recent_event_team(i));
    }
  }
}

// Try a few sensible locations for the bundled fixture (or an explicit
// `?xml=somefile.xml` query parameter).
async function fetchDefaultXml() {
  const explicit = new URLSearchParams(location.search).get('xml');
  const candidates = [];
  if (explicit) candidates.push(explicit);
  candidates.push(
    './matches/2647319.xml',
    './2647319.xml',
  );
  for (const path of candidates) {
    try {
      const res = await fetch(path);
      if (res.ok) return await res.text();
    } catch { /* try next */ }
  }
  throw new Error('Could not fetch the match XML. Copy an .xml file into web/ and use “Load path”, or use “Load XML”.');
}

// ── Render ─────────────────────────────────────────────────────
function render() {
  drawPitch();
  drawGoals();
  drawOffsideLine();
  if (!engine) return;

  // Fresh view every frame: the WASM heap may have grown during tick(),
  // which detaches any previously created TypedArray.
  const ptr = engine.get_render_buffer_ptr();
  const total = engine.get_total_entities();
  const players = engine.get_player_count();
  const buffer = new Float32Array(wasm.memory.buffer, ptr, total * ENTITY_STRIDE);

  for (let i = 0; i < players; i++) {
    const o = i * ENTITY_STRIDE;
    const [sx, sy] = toScreen(buffer[o], buffer[o + 1]);
    const team = buffer[o + 4];
    const shirt = buffer[o + 5];

    ctx.beginPath();
    ctx.arc(sx, sy, 7, 0, Math.PI * 2);
    ctx.fillStyle = team === 0 ? '#f2f2f2' : '#2f81f7';
    ctx.fill();
    ctx.strokeStyle = 'rgba(0,0,0,0.75)';
    ctx.lineWidth = 1.5;
    ctx.stroke();

    ctx.fillStyle = team === 0 ? '#0d1117' : '#ffffff';
    ctx.font = 'bold 8px ui-sans-serif, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(String(shirt), sx, sy);
  }

  // Ball (pinned to the final slot by the buffer layout).
  const bo = (total - 1) * ENTITY_STRIDE;
  const [bsx, bsy] = toScreen(buffer[bo], buffer[bo + 1]);
  const bz = buffer[bo + 2];

  if (bz > 0.05) {
    const shadowSize = Math.max(2, 4 - bz * 0.3);
    ctx.beginPath();
    ctx.ellipse(bsx + bz * 1.2, bsy + bz * 1.2, shadowSize * 1.5, shadowSize, 0, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(0,0,0,0.4)';
    ctx.fill();
  }

  const ballRadius = 3.5 + Math.min(bz * 0.8, 4.0);
  ctx.beginPath();
  ctx.arc(bsx, bsy - bz * 3.0, ballRadius, 0, Math.PI * 2);
  ctx.fillStyle = '#ffea00';
  ctx.fill();
  ctx.strokeStyle = '#222';
  ctx.lineWidth = 1;
  ctx.stroke();

  els.altitude.textContent = bz.toFixed(1);
}

function drawGoals() {
  const depth = 1.9; // metres of net behind the line
  const halfW = 3.66;
  for (const side of [-1, 1]) {
    const x0 = side * (PITCH_W_M / 2);
    const x1 = side * (PITCH_W_M / 2 + depth);
    const [sx0, syT] = toScreen(x0, halfW);
    const [sx1, syB] = toScreen(x1, -halfW);
    const left = Math.min(sx0, sx1), top = Math.min(syT, syB);
    const w = Math.abs(sx1 - sx0), h = Math.abs(syB - syT);
    ctx.save();
    ctx.fillStyle = 'rgba(255,255,255,0.05)';
    ctx.fillRect(left, top, w, h);
    ctx.strokeStyle = 'rgba(255,255,255,0.55)';
    ctx.lineWidth = 2;
    ctx.strokeRect(left, top, w, h);
    // Net mesh.
    ctx.strokeStyle = 'rgba(255,255,255,0.18)';
    ctx.lineWidth = 1;
    const cols = 5, rows = 4;
    for (let i = 1; i < cols; i++) {
      const gx = left + (w * i) / cols;
      ctx.beginPath(); ctx.moveTo(gx, top); ctx.lineTo(gx, top + h); ctx.stroke();
    }
    for (let j = 1; j < rows; j++) {
      const gy = top + (h * j) / rows;
      ctx.beginPath(); ctx.moveTo(left, gy); ctx.lineTo(left + w, gy); ctx.stroke();
    }
    ctx.restore();
  }
}

function drawOffsideLine() {
  if (!engine) return;
  const x = engine.get_offside_line();
  if (Number.isNaN(x)) return;
  const [sx] = toScreen(x, 0);
  ctx.save();
  ctx.setLineDash([7, 7]);
  ctx.strokeStyle = 'rgba(255, 90, 90, 0.75)';
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.moveTo(sx, OFFSET_Y);
  ctx.lineTo(sx, OFFSET_Y + PITCH_PX_H);
  ctx.stroke();
  ctx.restore();
}

function updateStats() {
  if (!engine) return;
  els.score.textContent = `${engine.get_score_home()} – ${engine.get_score_away()}`;
  const ph = engine.get_possession_home();
  els.posshome.style.width = `${(ph * 100).toFixed(0)}%`;
  els.possaway.style.width = `${((1 - ph) * 100).toFixed(0)}%`;
  els.possval.textContent = `${(ph * 100).toFixed(0)}%`;
}

// ── Main loop (fixed-step accumulator) ─────────────────────────
function loop(now) {
  const frame = Math.min((now - lastFrame) / 1000, 0.1);
  lastFrame = now;

  if (frame > 0) {
    const inst = 1 / frame;
    fpsSmooth = fpsSmooth === 0 ? inst : fpsSmooth * 0.9 + inst * 0.1;
    els.fps.textContent = fpsSmooth.toFixed(0);
  }

  if (engine && playing) {
    accumulator += frame * speed;
    let steps = 0;
    // Cap catch-up steps to avoid a spiral of death on slow frames.
    while (accumulator >= FIXED_DT && steps < 8) {
      engine.tick(FIXED_DT);
      accumulator -= FIXED_DT;
      steps++;
    }
    if (accumulator > FIXED_DT * 8) accumulator = 0;

    const t = engine.get_sim_time();
    const mins = String(Math.floor(t / 60)).padStart(2, '0');
    const secs = String(Math.floor(t % 60)).padStart(2, '0');
    els.timer.textContent = `${mins}:${secs}`;

    updateEvents();
    updateStats();

    // Full time: stop advancing so the pitch freezes on a finished match.
    if (engine.get_is_finished()) {
      playing = false;
      els.playpause.textContent = '▶ Play';
      setStatus('Full time — all events played.');
    }
  }

  render();
  requestAnimationFrame(loop);
}
els.playpause.addEventListener('click', () => {
  playing = !playing;
  els.playpause.textContent = playing ? '⏸ Pause' : '▶ Play';
});
els.restart.addEventListener('click', () => {
  if (lastXml) { try { startEngine(lastXml); } catch (e) { setStatus(String(e)); } }
});
els.speed.addEventListener('input', () => {
  speed = parseFloat(els.speed.value);
  els.speedval.textContent = `${speed.toFixed(2)}×`;
});
els.file.addEventListener('change', async (ev) => {
  const file = ev.target.files && ev.target.files[0];
  if (!file) return;
  try {
    startEngine(await file.text());
  } catch (e) {
    setStatus(`Failed to load ${file.name}: ${e}`);
  }
});
els.loadpath.addEventListener('click', loadFromPath);
els.back10.addEventListener('click', () => seekBy(-600));
els.fwd10.addEventListener('click', () => seekBy(600));
els.browse.addEventListener('click', browseDirectory);

function seekBy(seconds) {
  if (!engine) return;
  const target = engine.get_sim_time() + seconds;
  const dur = engine.get_duration_secs();
  engine.seek_to(Math.max(0, Math.min(dur, target)));
  lastEventSeq = -1;
  els.feed.innerHTML = '';
  setStatus(`Jumped to ${Math.floor(Math.max(0, target) / 60)} min`);
}
els.xmlpath.addEventListener('keydown', (ev) => {
  if (ev.key === 'Enter') loadFromPath();
});

async function loadFromPath() {
  const path = els.xmlpath.value.trim();
  if (!path) return;
  try {
    const res = await fetch(path);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    startEngine(await res.text());
  } catch (e) {
    setStatus(`Failed to load ${path}: ${e}`);
  }
}

// ── Directory browser ──────────────────────────────────────────
// Pick a folder, list the .xml files inside it, and load the one the user
// clicks. Uses the File System Access API where available (Chromium) and
// falls back to a webkitdirectory input elsewhere.
async function browseDirectory() {
  if (window.showDirectoryPicker) {
    try {
      const dirHandle = await window.showDirectoryPicker({ mode: 'read' });
      const list = [];
      for await (const entry of dirHandle.values()) {
        if (entry.kind === 'file' && entry.name.toLowerCase().endsWith('.xml')) {
          list.push({ name: entry.name, getFile: () => entry.getFile() });
        }
      }
      showFileList(list);
      return;
    } catch (e) {
      if (e && e.name === 'AbortError') return; // user cancelled the picker
      // Otherwise fall through to the input fallback below.
    }
  }
  const input = document.createElement('input');
  input.type = 'file';
  input.setAttribute('webkitdirectory', '');
  input.setAttribute('directory', '');
  input.addEventListener('change', () => {
    const list = Array.from(input.files || [])
      .filter((f) => f.name.toLowerCase().endsWith('.xml'))
      .map((f) => ({ name: f.name, getFile: async () => f }));
    showFileList(list);
  });
  input.click();
}

function showFileList(list) {
  if (!list.length) {
    setStatus('No .xml files found in that folder.');
    return;
  }
  els.filelist.innerHTML = '';
  for (const item of list) {
    const btn = document.createElement('button');
    btn.textContent = item.name;
    btn.addEventListener('click', async () => {
      try {
        const file = await item.getFile();
        startEngine(await file.text());
      } catch (e) {
        setStatus(`Failed to load ${item.name}: ${e}`);
      }
    });
    els.filelist.appendChild(btn);
  }
  setStatus(`${list.length} XML file(s) found — click one to load it.`);
}

// ── Match browser ──────────────────────────────────────────
// Fetch the build-time manifest (web/matches.json), render the
// scrollable match list, and auto-load the first match. An explicit
// ?xml= query parameter still takes precedence over the manifest.
async function loadMatchManifest() {
  const explicit = new URLSearchParams(location.search).get('xml');
  if (explicit) {
    try {
      const res = await fetch(explicit);
      if (res.ok) {
        startEngine(await res.text());
        return;
      }
    } catch { /* fall through to the manifest */ }
  }

  let entries = [];
  try {
    const res = await fetch('./matches.json');
    if (res.ok) entries = await res.json();
  } catch { /* no manifest — fall back to the bundled default */ }

  renderMatchList(entries);

  if (entries.length > 0) {
    await loadMatchFile(entries[0].file);
  } else {
    const xml = await fetchDefaultXml();
    startEngine(xml);
  }
}

function renderMatchList(entries) {
  els.matchList.innerHTML = '';
  if (!entries.length) {
    const div = document.createElement('div');
    div.className = 'empty';
    div.textContent = 'No bundled matches found.';
    els.matchList.appendChild(div);
    return;
  }
  for (const entry of entries) {
    const card = document.createElement('div');
    card.className = 'match-card';
    card.dataset.file = entry.file;

    const fixture = document.createElement('div');
    fixture.className = 'fixture';
    fixture.textContent = `${entry.home_team_name} v ${entry.away_team_name}`;

    const meta = document.createElement('div');
    meta.className = 'meta';
    meta.textContent = `${entry.game_date} · Comp ${entry.competition_id} · ${entry.season}`;

    card.appendChild(fixture);
    card.appendChild(meta);
    card.addEventListener('click', () => loadMatchFile(entry.file));
    els.matchList.appendChild(card);
  }
}

async function loadMatchFile(file) {
  try {
    const res = await fetch(`./matches/${file}`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    startEngine(await res.text());
    document.querySelectorAll('.match-card').forEach((c) =>
      c.classList.toggle('active', c.dataset.file === file)
    );
  } catch (e) {
    setStatus(`Failed to load ${file}: ${e}`);
  }
}

async function main() {
  try {
    wasm = await init();
    await loadMatchManifest();
  } catch (e) {
    setStatus(String(e.message || e));
  }
  requestAnimationFrame(loop);
}

main();
