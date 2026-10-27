// God's Eye View renderer.
//
// Loads the WASM engine, advances it on a fixed 60 Hz accumulator, and draws
// the 22 players + ball directly from the engine's flat render buffer using a
// zero-copy Float32Array view into WASM linear memory.

import init, { WasmMatchEngine } from './pkg/match_engine.js';
import {
  ROLE,
  applyRoles,
  attackChannels,
  formationLabel,
  passingLaneCandidates,
  visionCone,
} from './analysis.js';
import {
  drawAnalysis,
  drawAttackChannels,
  drawCelebrationFx,
  drawEventFx,
  drawHeatmap,
  drawOffsideLine,
  drawPassingLanes,
  drawPassingNetwork,
  drawSubstitutionFx,
  drawVisionCone,
} from './overlays.js';
import { createSubstitutionTracker } from './transitions.js';
import { createCelebrationTracker, createFxTracker } from './sequences.js';
import { createMomentumTracker, createPassingNetwork, rowValue, seqNumber } from './graphs.js';
import { createHeatmap } from './heatmap.js';
import { createDribbleIndex, formatClock, zoneOf } from './dribbles.js';
import { drawDribbleLayer } from './dribbleOverlay.js';
import { createPlayerPanel } from './playerPanel.js';
import { createPassingLaneTracker } from './passingLanes.js';
import { createAttackChannelTracker } from './attackChannels.js';
import { createMinimap } from './minimap.js';
import { createHighlightTracker } from './highlights.js';
import { createEffectController } from './effects.js';

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
  halftimeBanner: document.getElementById('halftime-banner'),
  htScore: document.getElementById('ht-score'),
  score: document.getElementById('score'),
  posshome: document.getElementById('posshome'),
  possaway: document.getElementById('possaway'),
  possval: document.getElementById('possval'),
  back5: document.getElementById('back5'),
  fwd5: document.getElementById('fwd5'),
  browse: document.getElementById('browse'),
  shuffle: document.getElementById('shuffle-effects'),
  filelist: document.getElementById('filelist'),
  matchList: document.getElementById('match-list'),
  matchInfo: document.getElementById('match-info'),
  homeName: document.getElementById('homename'),
  awayName: document.getElementById('awayname'),
  analysisTeam: document.getElementById('analysis-team'),
  analysisTeamLabel: document.getElementById('analysis-team-label'),
  playerContext: document.getElementById('player-context'),
  stage: document.getElementById('stage'),
  zoom: document.getElementById('zoom'),
  zoomval: document.getElementById('zoomval'),
  resetView: document.getElementById('reset-view'),
  zoneAlpha: document.getElementById('zone-alpha'),
  zoneAlphaVal: document.getElementById('zone-alphaval'),
};

const ctx = els.canvas.getContext('2d');
const effectButtons = Array.from(document.querySelectorAll('[data-effect]'));
const effects = createEffectController();

/** Keep the effect bar's pressed/current state in sync with the controller. */
function syncEffectButtons() {
  const shuffleOn = effects.shuffleActive;
  for (const button of effectButtons) {
    const key = button.dataset.effect;
    button.setAttribute('aria-pressed', String(effects.isEnabled(key)));
    button.classList.toggle('shuffle-current', shuffleOn && effects.currentKey === key);
  }
  if (els.shuffle) els.shuffle.setAttribute('aria-pressed', String(shuffleOn));
}
syncEffectButtons();

// ── Dribble analysis: index, panel and pitch interaction state ─
// The index is a whole-match, seek-safe snapshot built once per load; the
// panel renders from it and reports intent (seek / selection) outward.
let dribbleIndex = null;
let zoneAlpha = 0.55;
let hoveredPlayer = null; // { team, shirt } of the body under the cursor
let hoverZone = null; // zone under the cursor while the layer is visible
let panning = null; // { x, y, panX, panY, moved } during a drag

const dribblePanel = createPlayerPanel(document.getElementById('player-panel'), {
  /** Timeline strip: jump the sim to a dribble's match clock and pause. */
  onSeek: (matchSecs, dribble) => {
    seekTo(matchSecs, `Jumped to ${formatClock(dribble ? dribble.t : matchSecs)} — paused to inspect.`);
    playing = false;
    els.playpause.textContent = '▶ Play';
  },
  onSelection: (selection) => {
    if (!selection) hidePlayerMenu();
  },
  onToggleZones: () => {
    effects.setEnabled('dribbles', true);
    syncEffectButtons();
    setStatus('Dribble zones shown — click a zone on the pitch to filter the panel.');
  },
  onHorizonChange: (mode) => syncHorizonButtons(mode),
  status: setStatus,
});

// The stats-window toggle under the pitch mirrors the panel's own control:
// one mode, two places to reach it.
const horizonButtons = [...document.querySelectorAll('[data-horizon]')];
function syncHorizonButtons(mode) {
  for (const button of horizonButtons) {
    button.setAttribute('aria-pressed', String(button.dataset.horizon === mode));
  }
}
for (const button of horizonButtons) {
  button.addEventListener('click', () => dribblePanel?.setHorizonMode(button.dataset.horizon));
}
syncHorizonButtons(dribblePanel?.horizonMode ?? 'sofar');

/** Whole-match analysis log for the freshly loaded match (schema v1). */
function buildDribbleIndex() {
  dribbleIndex = null;
  if (typeof engine?.get_analysis_json !== 'function') return;
  try {
    dribbleIndex = createDribbleIndex(JSON.parse(engine.get_analysis_json()));
    dribblePanel?.setIndex(dribbleIndex, 0);
  } catch (e) {
    dribbleIndex = null;
    console.warn('Dribble analysis unavailable:', e);
  }
}

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

// ── View transform (zoom + pan) ────────────────────────────────
// One Canvas2D transform wraps every draw call — pitch, overlays and
// entities move together. At rest (zoom 1, pan 0) it is the identity, so
// `toScreen` output is bit-for-bit what it was before zooming existed.
const view = { zoom: 1, panX: 0, panY: 0 };
const VIEW_ZOOM_MAX = 2.5;

/** Apply the view transform to the pitch context (call once per frame). */
function applyViewTransform() {
  if (view.zoom === 1 && view.panX === 0 && view.panY === 0) {
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    return;
  }
  ctx.setTransform(
    view.zoom, 0, 0, view.zoom,
    CX + view.panX - CX * view.zoom,
    CY + view.panY - CY * view.zoom,
  );
}

/** Keep the viewport's centre over the pitch so the field stays reachable. */
function clampView() {
  view.zoom = Math.min(VIEW_ZOOM_MAX, Math.max(1, view.zoom));
  if (view.zoom === 1) {
    view.panX = 0;
    view.panY = 0;
    return;
  }
  const bx = CX - view.panX / view.zoom;
  const by = CY - view.panY / view.zoom;
  const mx = PITCH_PX_W * 0.15;
  const my = PITCH_PX_H * 0.15;
  const cx = Math.min(OFFSET_X + PITCH_PX_W + mx, Math.max(OFFSET_X - mx, bx));
  const cy = Math.min(OFFSET_Y + PITCH_PX_H + my, Math.max(OFFSET_Y - my, by));
  view.panX = (CX - cx) * view.zoom;
  view.panY = (CY - cy) * view.zoom;
}

/**
 * Canvas pixels → world metres: undo the view transform, then the base
 * screen → world mapping. Exact inverse of `toScreen` + the view wrap.
 */
function fromScreen(px, py) {
  const sx = (px - CX - view.panX) / view.zoom + CX;
  const sy = (py - CY - view.panY) / view.zoom + CY;
  return [
    (sx - OFFSET_X) / PX_PER_M - PITCH_W_M / 2,
    PITCH_H_M / 2 - (sy - OFFSET_Y) / PX_PER_M,
  ];
}

/** Canvas pixels → base-screen pixels (undo the view transform only). */
function toBaseScreen(px, py) {
  return [
    (px - CX - view.panX) / view.zoom + CX,
    (py - CY - view.panY) / view.zoom + CY,
  ];
}

/** Pointer client coordinates → canvas pixel coordinates (CSS-scaled). */
function pointerToCanvas(ev) {
  const rect = els.canvas.getBoundingClientRect();
  return [
    (ev.clientX - rect.left) * (els.canvas.width / rect.width),
    (ev.clientY - rect.top) * (els.canvas.height / rect.height),
  ];
}

function drawPitch() {
  // Dark, desaturated base.
  ctx.fillStyle = '#14261a';
  ctx.fillRect(0, 0, els.canvas.width, els.canvas.height);

  // Mowing stripes.
  ctx.fillStyle = 'rgba(255,255,255,0.025)';
  const stripeW = PITCH_PX_W / 12;
  for (let i = 0; i < 12; i += 2) {
    ctx.fillRect(OFFSET_X + i * stripeW, OFFSET_Y, stripeW, PITCH_PX_H);
  }

  // Pitch fill.
  ctx.fillStyle = '#1a3322';
  ctx.fillRect(OFFSET_X, OFFSET_Y, PITCH_PX_W, PITCH_PX_H);

  ctx.strokeStyle = 'rgba(255,255,255,0.62)';
  ctx.lineWidth = 1.5;
  ctx.strokeRect(OFFSET_X, OFFSET_Y, PITCH_PX_W, PITCH_PX_H);

  ctx.beginPath();
  ctx.moveTo(CX, OFFSET_Y);
  ctx.lineTo(CX, OFFSET_Y + PITCH_PX_H);
  ctx.stroke();

  // Centre circle + spot.
  ctx.beginPath();
  ctx.arc(CX, CY, CENTER_CIRCLE_R * PX_PER_M, 0, Math.PI * 2);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(CX, CY, 2.5, 0, Math.PI * 2);
  ctx.fill();

  // Penalty areas + arcs.
  const paW = PENALTY_DEPTH * PX_PER_M;
  const paH = PENALTY_WIDTH * PX_PER_M;
  const paTop = (els.canvas.height - paH) / 2;
  ctx.strokeRect(OFFSET_X, paTop, paW, paH);
  ctx.strokeRect(OFFSET_X + PITCH_PX_W - paW, paTop, paW, paH);

  // Goal areas.
  const gaW = GOAL_AREA_DEPTH * PX_PER_M;
  const gaH = GOAL_AREA_WIDTH * PX_PER_M;
  const gaTop = (els.canvas.height - gaH) / 2;
  ctx.strokeRect(OFFSET_X, gaTop, gaW, gaH);
  ctx.strokeRect(OFFSET_X + PITCH_PX_W - gaW, gaTop, gaW, gaH);

  // Penalty spots.
  const spotL = OFFSET_X + PENALTY_SPOT * PX_PER_M;
  const spotR = OFFSET_X + PITCH_PX_W - PENALTY_SPOT * PX_PER_M;
  ctx.beginPath();
  ctx.arc(spotL, CY, 2.5, 0, Math.PI * 2);
  ctx.arc(spotR, CY, 2.5, 0, Math.PI * 2);
  ctx.fill();

  // Penalty arcs (9.15m from spot, outside box).
  const arcR = 9.15 * PX_PER_M;
  ctx.beginPath();
  ctx.arc(spotL, CY, arcR, -Math.PI / 2, Math.PI / 2);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(spotR, CY, arcR, Math.PI / 2, (3 * Math.PI) / 2);
  ctx.stroke();

  // Corner arcs.
  const cr = 1 * PX_PER_M;
  [[OFFSET_X, OFFSET_Y, 0, Math.PI / 2],
   [OFFSET_X + PITCH_PX_W, OFFSET_Y, Math.PI / 2, Math.PI],
   [OFFSET_X, OFFSET_Y + PITCH_PX_H, -Math.PI / 2, 0],
   [OFFSET_X + PITCH_PX_W, OFFSET_Y + PITCH_PX_H, Math.PI, (3 * Math.PI) / 2]].forEach(([cx, cy, a0, a1]) => {
    ctx.beginPath();
    ctx.arc(cx, cy, cr, a0, a1);
    ctx.stroke();
  });

  // Vignette.
  const grad = ctx.createRadialGradient(CX, CY, PITCH_PX_H * 0.3, CX, CY, Math.max(els.canvas.width, els.canvas.height) * 0.7);
  grad.addColorStop(0, 'rgba(0,0,0,0)');
  grad.addColorStop(1, 'rgba(0,0,0,0.35)');
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, els.canvas.width, els.canvas.height);
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

// ── Team overlay state ─────────────────────────────────────────
// Overlays follow possession: only one team is drawn at a time, with a short
// hold and crossfade so turnovers never flicker the whole analysis layer.
let playerRoles = [];
let lastRoleSeq = -1;
let formations = ['—', '—'];
let displayTeam = 0;
let overlayTeamPrev = 0;
let possessHold = 0;
let teamBlend = 1;
let lastTeamChip = '';
const TEAM_SWITCH_DELAY = 0.35; // seconds of stable possession before switching
const TEAM_FADE = 0.18; // crossfade duration, seconds
const heat = [createHeatmap(), createHeatmap()];
let lastHeatSample = -Infinity;
const HEAT_SAMPLE_DT = 0.25;
const minimap = createMinimap(
  document.getElementById('minimap'),
  document.getElementById('minimap-title'),
);
const highlighter = createHighlightTracker(
  document.getElementById('highlight'),
  document.getElementById('highlight-title'),
);
let offsideX = NaN;
let offsideAlpha = 0;
const OFFSIDE_SMOOTH_TAU = 0.35;
const OFFSIDE_FADE_TAU = 0.25;
const substitutions = createSubstitutionTracker();
const celebrations = createCelebrationTracker();
const fx = createFxTracker();
const momentum = createMomentumTracker();
const passingNet = createPassingNetwork();
const passingLanes = createPassingLaneTracker();
const attackChannelsFx = createAttackChannelTracker();
// Player-only heatmap (minimap).
const playerHeat = createHeatmap();
let chosenPlayerShirt = null;
let playerHeatTimer = 0;
let lastPlayerHeatTeam = -1;
let lastPlayerHeatSample = -Infinity;
// Live overlays: sample the geometry at 10 Hz, resolve endpoints every frame.
let lastLaneCompute = -Infinity;
let lastLaneData = [];
let lastChannelCompute = -Infinity;
let lastChannelData = [];
// Frame-to-frame velocity per team:shirt (the render buffer has no velocity).
const playerVelocity = new Map();
let visionAlpha = 0;
let lastVisionCarrier = '';
let lastVisionCone = null;
let visionFocus = null;
// Per-team last passer (link consecutive same-team rows into network edges).
const lastPasser = [{ id: '', pos: null }, { id: '', pos: null }];
// Cursor for the sequence tracker (goal mob / save puff / flag / banner
// triggers). Distinct from lastEventSeq (feed rendering). The processed-key
// set makes the scan seek-safe when the rolling feed window shifts indices.
let lastSeqSeen = -1;
const processedEvents = new Set();
let sequencesPrimed = false;
// Latest snapshots for sequence triggers (goal mob needs live positions).
let lastPlayers = [];
let lastBall = null;
// Half-time renderer choreography (engine keeps placing the kickoff; we
// overlay a walk-to-tunnel during the gap and snap to engine positions once
// the kickoff set piece arms).
let halftimeActive = false;
let halftimeStartSim = 0;
let halftimeStartPositions = [];
let halftimeBannerUntil = 0;
let halftimeKickoffReady = false;
let lastPeriod = 1;

function resetSequenceCursor() {
  lastSeqSeen = -1;
  processedEvents.clear();
  sequencesPrimed = false;
}

function resetOverlayState() {
  heat[0].reset();
  heat[1].reset();
  lastHeatSample = -Infinity;
  offsideX = NaN;
  offsideAlpha = 0;
  const possession = typeof engine?.get_possession_team === 'function'
    ? engine.get_possession_team()
    : -1;
  displayTeam = possession === 1 ? 1 : 0;
  overlayTeamPrev = displayTeam;
  possessHold = 0;
  teamBlend = 1;
  lastTeamChip = '';
  lastRoleSeq = -1;
  playerRoles = [];
  formations = ['—', '—'];
}

/** Accumulate both teams' positions at most every 0.25 s of match time. */
function sampleHeat(players, simTime) {
  if (simTime - lastHeatSample < HEAT_SAMPLE_DT) return;
  lastHeatSample = simTime;
  const home = [];
  const away = [];
  for (const p of players) {
    if (p.team === 0) home.push(p);
    else if (p.team === 1) away.push(p);
  }
  heat[0].sample(home);
  heat[1].sample(away);
}

/** Sample a single randomly chosen player's positions into playerHeat. */
function samplePlayerHeat(players, simTime) {
  // Reset when followed team changes
  if (lastPlayerHeatTeam !== displayTeam) {
    lastPlayerHeatTeam = displayTeam;
    chosenPlayerShirt = null;
    playerHeat.reset();
    playerHeatTimer = simTime;
  }
  if (chosenPlayerShirt === null) {
    // pick a random outfield player from the followed team
    const teamPlayers = players.filter((p) => p.team === displayTeam);
    if (teamPlayers.length === 0) return;
    const pick = teamPlayers[Math.floor(Math.random() * teamPlayers.length)];
    chosenPlayerShirt = pick.shirt;
    playerHeat.reset();
    playerHeatTimer = simTime;
  }
  // rotate chosen player every ~45s of sim time
  if (simTime - playerHeatTimer > 45) {
    const teamPlayers = players.filter((p) => p.team === displayTeam);
    if (teamPlayers.length > 0) {
      const pick = teamPlayers[Math.floor(Math.random() * teamPlayers.length)];
      chosenPlayerShirt = pick.shirt;
      playerHeat.reset();
    }
    playerHeatTimer = simTime;
  }
  // expose to minimap data
  if (minimap && minimap.data) {
    minimap.data.playerHeat = playerHeat;
    minimap.data.chosenPlayerShirt = chosenPlayerShirt;
  }
  if (simTime - lastPlayerHeatSample < HEAT_SAMPLE_DT) return;
  lastPlayerHeatSample = simTime;
  const chosen = players.find((p) => p.team === displayTeam && p.shirt === chosenPlayerShirt);
  if (chosen) playerHeat.sample([chosen]);
  else if (chosenPlayerShirt !== null) {
    // Substituted or sent off: pick a fresh subject on the next sample.
    chosenPlayerShirt = null;
    playerHeat.reset();
  }
}

/** Follow possession with hysteresis and a short crossfade. */
function updateFollowedTeam(possessionTeam, dt) {
  if (possessionTeam === 0 || possessionTeam === 1) {
    if (possessionTeam === displayTeam) {
      possessHold = 0;
    } else {
      possessHold += dt;
      if (possessHold >= TEAM_SWITCH_DELAY) {
        overlayTeamPrev = displayTeam;
        displayTeam = possessionTeam;
        possessHold = 0;
        teamBlend = 0;
        // Interception: a sustained change of possession ends the retained
        // passing sequence (the network hides immediately while the old team
        // is out of possession, and the streak resets on this switch).
        passingNet.breakRetention();
        // Transient overlays belong to the old team; never show them under
        // the new team's colours during the crossfade.
        passingLanes.reset();
        attackChannelsFx.reset();
        lastLaneData = [];
        lastChannelData = [];
      }
    }
  } else {
    possessHold = 0;
  }
  if (teamBlend < 1) teamBlend = Math.min(1, teamBlend + dt / TEAM_FADE);
}

function updateTeamChip() {
  if (!els.analysisTeam || !els.analysisTeamLabel) return;
  const team = displayTeam === 1 ? 1 : 0;
  const name = team === 0
    ? (els.homeName.textContent || 'Home')
    : (els.awayName.textContent || 'Away');
  const shape = formations[team];
  const label = shape && shape !== '—' ? `${name} · ${shape}` : name;
  if (label === lastTeamChip && els.analysisTeam.dataset.team === String(team)) return;
  lastTeamChip = label;
  els.analysisTeam.dataset.team = String(team);
  els.analysisTeamLabel.textContent = label;
}

/** Draw the overlay layer for the followed team (both sides while fading). */
function drawOverlays(players, possessionTeam, setPieceActive, dt, ball) {
  const teams = [];
  if (teamBlend < 1 && overlayTeamPrev !== displayTeam) {
    teams.push({ team: overlayTeamPrev, alpha: 1 - teamBlend });
  }
  teams.push({ team: displayTeam, alpha: teamBlend < 1 ? teamBlend : 1 });

  const rect = { x: OFFSET_X, y: OFFSET_Y, w: PITCH_PX_W, h: PITCH_PX_H };
  if (effects.isVisible('heatmap')) {
    for (const { team, alpha } of teams) {
      drawHeatmap(ctx, rect, heat[team], team, alpha);
    }
  }

  // Dribble zones: drawn for the panel's current scope — the followed team
  // by default, one player once a player is selected — and time-aware, so
  // zones and markers fill in as the match plays.
  if (effects.isVisible('dribbles')) {
    const stats = dribblePanel?.layerStats();
    if (stats) {
      drawDribbleLayer(ctx, toScreen, stats, dribblePanel.layerTeam(), {
        alpha: zoneAlpha,
        hoverZone,
        selectedZone: dribblePanel.selectedZone ?? null,
      });
    }
  }

  // Passing network: followed team, visible through the crossfade, and shown
  // until possession is lost (the streak is broken on the team switch).
  const netAlpha = Math.max(0, teamBlend);
  const stillInPossession = possessionTeam === displayTeam;
  if (effects.isVisible('network') && netAlpha > 0.01 && stillInPossession) {
    const snap = passingNet.snapshot(8, displayTeam);
    // Resolve nodes on the live bodies so links move with the players.
    const livePositions = new Map();
    for (const p of players) livePositions.set(`${p.team}:${p.shirt}`, { x: p.x, y: p.y });
    drawPassingNetwork(ctx, toScreen, snap, netAlpha, displayTeam, livePositions);
  }

  // Offside line: open play only, for the followed team in possession.
  const rawOffside = typeof engine?.get_offside_line === 'function'
    ? engine.get_offside_line()
    : NaN;
  const showOffside =
    (possessionTeam === 0 || possessionTeam === 1) &&
    possessionTeam === displayTeam &&
    !setPieceActive &&
    Number.isFinite(rawOffside);
  if (showOffside) {
    if (!Number.isFinite(offsideX)) offsideX = rawOffside;
    else offsideX += (rawOffside - offsideX) * (1 - Math.exp(-dt / OFFSIDE_SMOOTH_TAU));
  }
  offsideAlpha += ((showOffside ? 1 : 0) - offsideAlpha) *
    (1 - Math.exp(-dt / OFFSIDE_FADE_TAU));
  if (!showOffside && offsideAlpha < 0.01) offsideAlpha = 0;
  if (effects.isVisible('offside') && offsideAlpha > 0.01) {
    drawOffsideLine(ctx, toScreen, offsideX, offsideAlpha, OFFSET_Y, OFFSET_Y + PITCH_PX_H);
  }

  if (effects.isVisible('analysis')) {
    for (const { team, alpha } of teams) {
      // The animated passing lanes + vision cone replace the static thread and
      // triangle, so the analysis layer stays legible.
      drawAnalysis(ctx, toScreen, players, team, alpha, ball, possessionTeam, { links: false });
    }
  }
}

/** Role cache: refresh only when an event (e.g. a substitution) changes. */
function refreshRoles(players) {
  const seq = typeof engine.get_event_seq === 'function'
    ? seqNumber(engine.get_event_seq())
    : -1;
  if (seq === lastRoleSeq && playerRoles.length === players.length) {
    // Player snapshots are rebuilt every frame; always re-apply the cache.
    applyRoles(players, playerRoles);
    return;
  }

  lastRoleSeq = seq;
  const roles = new Array(players.length).fill(ROLE.UNKNOWN);
  let anyKnown = false;
  if (typeof engine.get_player_role === 'function') {
    for (let i = 0; i < players.length; i++) {
      const role = engine.get_player_role(i) | 0;
      roles[i] = role;
      if (role) anyKnown = true;
    }
  }
  if (!anyKnown) inferRoles(players, roles);

  playerRoles = roles;
  applyRoles(players, roles);
  formations = [0, 1].map((team) =>
    formationLabel(players.filter((p) => p.team === team)),
  );
}

/** Position-only fallback for an older wasm package without role getters. */
function inferRoles(players, roles) {
  for (const team of [0, 1]) {
    const dir = team === 0 ? 1 : -1;
    const squad = players
      .map((p, i) => ({ p, i }))
      .filter((entry) => entry.p.team === team)
      .sort((a, b) => b.p.x * dir - a.p.x * dir);
    const n = squad.length;
    squad.forEach((entry, rank) => {
      if (rank >= n - 1) roles[entry.i] = ROLE.GK;
      else if (rank >= n - 5) roles[entry.i] = ROLE.DEF;
      else if (rank < 3) roles[entry.i] = ROLE.FWD;
      else roles[entry.i] = ROLE.MID;
    });
  }
}

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
  resetSequenceCursor();
  resetOverlayState();
  halftimeActive = false;
  halftimeStartPositions = [];
  halftimeKickoffReady = false;
  lastPeriod = typeof engine.get_current_period === 'function' ? engine.get_current_period() : 1;
  if (els.halftimeBanner) els.halftimeBanner.classList.remove('show');
  substitutions.reset();
  celebrations.reset();
  fx.reset();
  momentum.reset();
  passingNet.reset();
  passingLanes.reset();
  attackChannelsFx.reset();
  playerHeat.reset();
  chosenPlayerShirt = null;
  playerHeatTimer = 0;
  lastPlayerHeatTeam = -1;
  lastPlayerHeatSample = -Infinity;
  lastLaneCompute = -Infinity;
  lastLaneData = [];
  lastChannelCompute = -Infinity;
  lastChannelData = [];
  playerVelocity.clear();
  visionAlpha = 0;
  lastVisionCarrier = '';
  lastVisionCone = null;
  visionFocus = null;
  if (highlighter) { highlighter.buffer.length = 0; }
  lastPasser[0] = { id: '', pos: null };
  lastPasser[1] = { id: '', pos: null };
  els.feed.innerHTML = '';
  els.splash.classList.remove('show');
  els.restart.disabled = false;
  // Label the HUD with the match metadata straight from the engine.
  els.homeName.textContent = engine.get_home_team_name() || 'Home';
  els.awayName.textContent = engine.get_away_team_name() || 'Away';
  const meta = [engine.get_competition_id(), engine.get_season(), engine.get_game_date()]
    .filter(Boolean);
  els.matchInfo.textContent = meta.length ? meta.join(' · ') : '';
  // Fresh match → fresh whole-match dribble index; the panel resets with it.
  hidePlayerMenu();
  hoveredPlayer = null;
  hoverZone = null;
  buildDribbleIndex();
  updateClockLabel();
  updateStats();
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
  const seq = seqNumber(engine.get_event_seq());
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

  // Highlight replay: collect notable events with a frozen frame.
  if (highlighter && count > 0) {
    const i = count - 1;
    const notable = engine.get_recent_event_notable(i);
    const type = engine.get_recent_event_type(i);
    // Important event types: goals, shots, corners, penalties, fouls in final third.
    const important = notable || type === 16 || type === 13 || type === 14 || type === 15 || type === 10 || type === 4;
    if (important) {
      const t = engine.get_recent_event_time(i);
      const label = engine.get_recent_event_label(i);
      const team = engine.get_recent_event_team(i);
      const x = engine.get_last_event_origin_x();
      const y = engine.get_last_event_origin_y();
      highlighter.push({ t, type, label, team, players: lastPlayers.map((p) => ({ x: p.x, y: p.y, team: p.team, shirt: p.shirt })), ball: lastBall ? { x: lastBall.x, y: lastBall.y } : null, x, y });
    }
  }
  // Track minimap data.
  if (minimap && minimap.data) {
    minimap.data.heat.home = heat[0];
    minimap.data.heat.away = heat[1];
    if (count > 0) {
      const i = count - 1;
      const type = engine.get_recent_event_type(i);
      const team = engine.get_recent_event_team(i);
      const x = engine.get_last_event_origin_x();
      const y = engine.get_last_event_origin_y();
      const teamKey = team === 0 ? 'home' : 'away';
      if (type >= 13 && type <= 16) {
        minimap.data.shots[teamKey].push({ x, y, targetY: 0 });
      } else if (type === 1 || type === 2) {
        minimap.data.passes[teamKey].push({ x, y });
        if (Math.abs(x) > 35) minimap.data.attacks[teamKey].push({ x, y });
      } else if (type === 7 || type === 8) {
        minimap.data.tackles[teamKey].push({ x, y });
      } else if (type === 4) {
        minimap.data.setPieces.push({ x, y, team });
      }
      // Recoveries: type 8 (interception) already counted; add ball recoveries from type 74 (blocked pass).
      if (type === 74) minimap.data.recoveries[teamKey].push({ x, y });
      // Progressive passes: passes that move the ball >10m forward.
      if ((type === 1 || type === 2) && Math.abs(x) > 10) {
        minimap.data.progressive[teamKey].push({ x, y });
      }
    }
  }

  if (count > 0) {
    const i = count - 1;
    if (engine.get_recent_event_notable(i)) {
      showSplash(engine.get_recent_event_label(i), engine.get_recent_event_team(i));
    }
  }
}

// ── Scripted sequences ─────────────────────────────────
// Keys off fresh feed entries: goal mob, save puff, post ping, offside flag,
// card banner. Positions come from the live snapshots, never the feed.
function triggerSequences() {
  if (!engine || typeof engine.get_event_seq !== 'function') return;
  const seq = seqNumber(engine.get_event_seq());
  if (seq === lastSeqSeen) return;
  lastSeqSeen = seq;
  // Walk every entry not processed yet (a tick can dispatch several
  // same-second rows). On the first frame after load/seek, older rows are
  // history: mark them as seen but never replay their celebrations.
  const count = engine.get_recent_event_count();
  const firstRun = !sequencesPrimed;
  sequencesPrimed = true;
  for (let i = 0; i < count; i++) {
    const type = engine.get_recent_event_type(i);
    const team = engine.get_recent_event_team(i);
    const t = engine.get_recent_event_time(i);
    const label = engine.get_recent_event_label(i);
    const key = `${type}:${t}:${label}:${team}`;
    if (processedEvents.has(key)) continue;
    processedEvents.add(key);
    if (firstRun && i < count - 1) continue;
    const playerId = typeof engine.get_recent_event_player === 'function'
      ? engine.get_recent_event_player(i)
      : '';
    const idx = playerId && typeof engine.get_player_index_by_id === 'function'
      ? engine.get_player_index_by_id(playerId)
      : -1;
    const p = idx >= 0 && idx < lastPlayers.length ? lastPlayers[idx] : null;
    // Momentum strip + passing network observe every row.
    const { dv, big } = rowValue(type, label);
    momentum.observe(`${seq}:${i}`, [{ t, dv: team === 2 ? 0 : dv, label, type, team }]);
    void big;
    // Passing links: consecutive completed rows by the same team.
    if ((type === 1 || type === 61 || type === 83) && p && (team === 0 || team === 1)) {
      const prev = lastPasser[team];
      if (prev.id && prev.id !== playerId && prev.pos) {
        passingNet.observePass(
          { ...prev.pos, team, shirt: prev.shirt },
          { ...p, shirt: p.shirt },
        );
      }
      lastPasser[team] = { id: playerId, shirt: p.shirt, pos: { x: p.x, y: p.y } };
    } else if (team === 0 || team === 1) {
      lastPasser[team] = { id: '', pos: null };
    }
    if (type === 16 && p) {
      // Goal: scorer + nearest mates converge.
      const mates = lastPlayers
        .filter((q, j) => q && q.team === p.team && j !== idx)
        .map((q) => ({ ...q, d: Math.hypot(q.x - p.x, q.y - p.y) }))
        .sort((a, b) => a.d - b.d)
        .slice(0, 3);
      celebrations.trigger({ ...p }, mates);
    } else if (type === 10 && lastBall) {
      fx.spawn('save', lastBall.x, lastBall.y);
    } else if (type === 14) {
      // Woodwork: ping at the nearest post to the ball.
      const gx = lastBall && lastBall.x >= 0 ? 52.5 : -52.5;
      const gy = lastBall && lastBall.y >= 0 ? 3.66 : -3.66;
      fx.spawn('post', gx, gy);
    } else if (type === 2 || type === 55) {
      // Offside: flag freeze at the ball (attackers hold their run).
      if (lastBall) fx.spawn('flag', lastBall.x, lastBall.y);
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
function render(dt = 0) {
  if (effects.update(dt)) syncEffectButtons();
  // Untransformed base fill: panned margins keep the page colour instead of
  // showing stale pixels, then the view transform carries every layer.
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.fillStyle = '#0d1117';
  ctx.fillRect(0, 0, els.canvas.width, els.canvas.height);
  applyViewTransform();
  drawPitch();
  drawGoals();
  if (!engine) return;

  // Fresh view every frame: the WASM heap may have grown during tick(),
  // which detaches any previously created TypedArray.
  const ptr = engine.get_render_buffer_ptr();
  const total = engine.get_total_entities();
  const playerCount = engine.get_player_count();
  const buffer = new Float32Array(wasm.memory.buffer, ptr, total * ENTITY_STRIDE);

  // Snapshot players once; both the overlays and the entity drawing use it.
  const players = [];
  for (let i = 0; i < playerCount; i++) {
    const o = i * ENTITY_STRIDE;
    const p = {
      x: buffer[o],
      y: buffer[o + 1],
      heading: buffer[o + 3],
      team: buffer[o + 4],
      shirt: buffer[o + 5],
      role: ROLE.UNKNOWN,
      dismissed: false,
    };
    if (typeof engine.get_is_player_dismissed === 'function') {
      try {
        p.dismissed = engine.get_is_player_dismissed(i);
      } catch { /* ignore */ }
    }
    players.push(p);
  }
  // Derive velocity from consecutive positions. The buffer carries heading
  // only, but the pass fan, corridor interception and run channels all need
  // real vx/vy to predict where bodies will be.
  const simNow = typeof engine.get_sim_time === 'function' ? engine.get_sim_time() : 0;
  // The analysis panel shows "so far" stats by default: hand it the clock
  // every frame. It re-renders only when a dribble boundary is crossed.
  dribblePanel?.setNow(simNow);
  for (const p of players) {
    const key = `${p.team}:${p.shirt}`;
    const prev = playerVelocity.get(key);
    if (prev && simNow > prev.t + 1e-4) {
      const dts = simNow - prev.t;
      p.vx = (p.x - prev.x) / dts;
      p.vy = (p.y - prev.y) / dts;
      // A seek or a teleport must not become a 200 m/s sprint.
      const sp = Math.hypot(p.vx, p.vy);
      if (sp > 12) {
        p.vx = (p.vx / sp) * 12;
        p.vy = (p.vy / sp) * 12;
      }
      p.speed = Math.hypot(p.vx, p.vy);
    } else {
      p.vx = 0;
      p.vy = 0;
      p.speed = 0;
    }
    playerVelocity.set(key, { x: p.x, y: p.y, t: simNow });
  }
  const bo = (total - 1) * ENTITY_STRIDE;
  const ball = { x: buffer[bo], y: buffer[bo + 1], z: buffer[bo + 2] };
  lastPlayers = players;
  lastBall = ball;
  // Half-time choreography detection.
  if (engine && typeof engine.get_current_period === 'function') {
    const curPeriod = engine.get_current_period();
    if (curPeriod > lastPeriod) {
      halftimeActive = true;
      halftimeKickoffReady = false;
      halftimeStartSim = engine.get_sim_time();
      halftimeStartPositions = players.map((p) => ({ x: p.x, y: p.y, team: p.team, shirt: p.shirt }));
      halftimeBannerUntil = halftimeStartSim + 6;
      if (els.htScore) els.htScore.textContent = `${engine.get_score_home()} – ${engine.get_score_away()}`;
    }
    lastPeriod = curPeriod;
  }
  if (halftimeActive) {
    const spActive = typeof engine.get_set_piece_active === 'function' ? engine.get_set_piece_active() : false;
    const spKind = typeof engine.get_set_piece_kind === 'function' ? engine.get_set_piece_kind() : '';
    if (spActive && spKind === 'kickoff') halftimeKickoffReady = true;
    if (halftimeKickoffReady) halftimeActive = false;
  }
  // Banner show/hide.
  if (els.halftimeBanner) {
    const simT = engine ? engine.get_sim_time() : 0;
    const show = halftimeActive && simT < halftimeBannerUntil;
    els.halftimeBanner.classList.toggle('show', show);
  }

  // Drive momentum / passing network / celebrations / FX from the event feed.
  // (triggerSequences is seek-safe and no-ops when the event cursor is unchanged.)
  try { triggerSequences(); } catch (e) { /* ignore */ }

  // Substitutions are a renderer-only choreography. The tracker spots a
  // (team, shirt) change in a slot and returns the two walking ghosts; the
  // live body is hidden only while its replacement is running on.
  const subFx = substitutions.update(players, dt);
  const hiddenSlots = substitutions.hiddenSlots();

  refreshRoles(players);

  const possessionTeam =
    typeof engine.get_possession_team === 'function' ? engine.get_possession_team() : -1;
  const setPieceActive =
    typeof engine.get_set_piece_active === 'function' ? engine.get_set_piece_active() : false;

  // Keep overlays anchored to bodies the viewer can actually see: no sub
  // ghosts, no sent-off walk-offs (still rendered, never analysed).
  const dismissed = new Set();
  if (typeof engine.get_is_player_dismissed === 'function') {
    for (let i = 0; i < players.length; i++) {
      if (engine.get_is_player_dismissed(i)) dismissed.add(i);
    }
  }
  const overlayPlayers = (hiddenSlots.size > 0 || dismissed.size > 0)
    ? players.filter((_, i) => !hiddenSlots.has(i) && !dismissed.has(i))
    : players;

  updateFollowedTeam(possessionTeam, dt);
  updateTeamChip();
  // The zone layer and the team lens follow possession unless the analyst
  // has pinned a player (that selection owns both the panel and the layer).
  if (dribblePanel && dribblePanel.view !== 'player') {
    dribblePanel.setTeam(displayTeam);
  }
  if (typeof engine.get_sim_time === 'function') {
    sampleHeat(overlayPlayers, engine.get_sim_time());
    samplePlayerHeat(overlayPlayers, engine.get_sim_time());
  }
  drawOverlays(overlayPlayers, possessionTeam, setPieceActive, dt, ball);

  // Live positions for overlays that must stay attached to the players.
  const liveByKey = new Map();
  for (const p of overlayPlayers) liveByKey.set(`${p.team}:${p.shirt}`, { x: p.x, y: p.y });

  const attackDir = displayTeam === 0 ? 1 : -1;
  const carrierTeam = displayTeam;
  const teamPlayers = overlayPlayers.filter((p) => p.team === carrierTeam);
  let carrier = null;
  let bestD = Infinity;
  for (const p of teamPlayers) {
    const d = Math.hypot(p.x - ball.x, p.y - ball.y);
    if (d < bestD) { bestD = d; carrier = p; }
  }
  const hasCarrier = carrier && bestD <= 2.5 && !setPieceActive;
  const overlayDt = playing ? dt : 0;

  // Vision cone: smooth fade in/out and reset when the carrier changes so the
  // wedge never jumps across the pitch.
  const carrierKey = hasCarrier ? `${carrierTeam}:${carrier.shirt}` : '';
  if (carrierKey !== lastVisionCarrier) {
    lastVisionCarrier = carrierKey;
    visionAlpha = 0;
  }
  visionAlpha += ((hasCarrier ? 1 : 0) - visionAlpha) * (1 - Math.exp(-overlayDt / 0.18));
  if (hasCarrier) {
    // Passing options are re-scored at 10 Hz; endpoints resolve live.
    if (simNow - lastLaneCompute >= 0.1) {
      lastLaneCompute = simNow;
      lastLaneData = passingLaneCandidates(overlayPlayers, ball, carrierTeam, attackDir);
    }
    // Face the best available pass (not the ball at the carrier's feet), and
    // ease the focus so the cone never snaps between receivers.
    let focusTarget = null;
    let bestP = 0.45;
    for (const lane of lastLaneData) {
      if ((lane.p ?? 0) > bestP) {
        bestP = lane.p ?? 0;
        const key = lane.player ? `${lane.player.team}:${lane.player.shirt}` : null;
        focusTarget = (key && liveByKey.get(key)) || lane.to;
      }
    }
    if (focusTarget) {
      if (!visionFocus) visionFocus = { x: focusTarget.x, y: focusTarget.y };
      else {
        const k = 1 - Math.exp(-overlayDt / 0.18);
        visionFocus.x += (focusTarget.x - visionFocus.x) * k;
        visionFocus.y += (focusTarget.y - visionFocus.y) * k;
      }
    } else {
      visionFocus = null;
    }
    const teammates = teamPlayers.filter((p) => p !== carrier);
    const opponents = overlayPlayers.filter((p) => p.team !== carrierTeam);
    lastVisionCone = visionCone(carrier, teammates, opponents, attackDir, visionFocus);
  } else {
    lastLaneData = [];
    visionFocus = null;
  }
  if (effects.isVisible('visionCone') && lastVisionCone && visionAlpha > 0.01) {
    drawVisionCone(ctx, toScreen, lastVisionCone, teamBlend * visionAlpha);
  }

  // Run channels are re-scored at 10 Hz and anchored to live runners.
  const ballProgress = ball.x * attackDir;
  if (!setPieceActive && ballProgress > 15 && possessionTeam === displayTeam) {
    if (simNow - lastChannelCompute >= 0.1) {
      lastChannelCompute = simNow;
      lastChannelData = attackChannels(
        overlayPlayers,
        ball,
        displayTeam,
        attackDir,
        overlayPlayers.filter((p) => p.team !== displayTeam),
      );
    }
  } else {
    lastChannelData = [];
  }

  // Pausing the sim freezes the transient overlays so an analyst can read
  // them, while the 10 Hz samplers keep the last geometry.
  const laneSets = passingLanes.updateLive(overlayDt, lastLaneData, (lane) => {
    const key = lane.player ? `${lane.player.team}:${lane.player.shirt}` : null;
    const live = key ? liveByKey.get(key) : null;
    return {
      from: { x: ball.x, y: ball.y },
      to: live ?? lane.to,
    };
  });
  if (effects.isVisible('passingLanes')) {
    drawPassingLanes(ctx, toScreen, laneSets, teamBlend);
  }
  const channelSet = attackChannelsFx.updateLive(overlayDt, lastChannelData, (ch) => {
    const key = ch.player ? `${ch.player.team}:${ch.player.shirt}` : null;
    return (key && liveByKey.get(key)) || ch.from;
  });
  if (effects.isVisible('attackChannels')) {
    drawAttackChannels(ctx, toScreen, channelSet, teamBlend);
  }

  if (minimap) minimap.update(performance.now() / 1000);
  if (highlighter) highlighter.update(performance.now() / 1000, engine ? engine.get_sim_time() : 0);

  // The walk-off / walk-on pair sits above the tactical wash but below the
  // ball, so a substitute crossing the touchline never hides the ball.
  drawSubstitutionFx(ctx, toScreen, subFx);

  // Scripted pitch theatre (goal mob, save puff, post ping, offside flag).
  const celebFx = celebrations.update(players, ball, dt);
  if (celebFx) drawCelebrationFx(ctx, toScreen, celebFx);
  drawEventFx(ctx, toScreen, fx.update(dt));

  for (let i = 0; i < players.length; i++) {
    if (hiddenSlots.has(i)) continue;
    const p = players[i];
    let px = p.x, py = p.y;
    if (halftimeActive && halftimeStartPositions[i]) {
      const simT = engine.get_sim_time();
      const span = Math.max(1, halftimeBannerUntil - halftimeStartSim); // ~6s walk
      // walk duration = banner duration window; extend to gap
      const walkDur = Math.max(6, (halftimeStartPositions.length ? 12 : 6));
      const progress = Math.min(1, Math.max(0, (simT - halftimeStartSim) / walkDur));
      const sp = halftimeStartPositions[i];
      const tunnelX = p.team === 0 ? -56 : 56;
      const tunnelY = (p.shirt % 11) * 5 - 25; // spread
      px = sp.x + (tunnelX - sp.x) * progress;
      py = sp.y + (tunnelY - sp.y) * progress;
      // small vertical bob
      py += Math.sin(progress * Math.PI * 4 + i) * 0.4 * (1 - progress);
    }
    const [sx, sy] = toScreen(px, py);

    // Soft foot shadow lifts the player off the pitch.
    ctx.beginPath();
    ctx.ellipse(sx + 1.2, sy + 1.8, 4.6, 2.6, 0, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(0,0,0,0.28)';
    ctx.fill();

    // Player marker: 5 px ≈ 0.54 m radius at this pitch scale, much closer
    // to a real body than the old 7 px disc.
    ctx.beginPath();
    ctx.arc(sx, sy, 5, 0, Math.PI * 2);
    ctx.fillStyle = p.team === 0 ? '#f2f2f2' : '#2f81f7';
    ctx.fill();
    ctx.strokeStyle = 'rgba(0,0,0,0.75)';
    ctx.lineWidth = 1.2;
    ctx.stroke();

    // GK marker: small gloves ring.
    // (role not in player snapshot here; skip to keep simple)

    ctx.fillStyle = p.team === 0 ? '#0d1117' : '#ffffff';
    ctx.font = 'bold 8px ui-sans-serif, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(String(p.shirt), sx, sy);
  }

  // Ball (pinned to the final slot by the buffer layout).
  const [bsx, bsy] = toScreen(ball.x, ball.y);
  const bz = ball.z;

  if (bz > 0.05) {
    const shadowSize = Math.max(2, 4 - bz * 0.3);
    ctx.beginPath();
    ctx.ellipse(bsx + bz * 1.2, bsy + bz * 1.2, shadowSize * 1.5, shadowSize, 0, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(0,0,0,0.4)';
    ctx.fill();
  }

  const ballRadius = 2.8 + Math.min(bz * 0.6, 3.0);
  ctx.beginPath();
  ctx.arc(bsx, bsy - bz * 3.0, ballRadius, 0, Math.PI * 2);
  ctx.fillStyle = '#ffea00';
  ctx.fill();
  ctx.strokeStyle = '#222';
  ctx.lineWidth = 1;
  ctx.stroke();

  drawHoveredPlayer();

  els.altitude.textContent = bz.toFixed(1);
  try { drawMomentum(); } catch (e) { /* ignore */ }
}

/**
 * Ring + name over the body under the cursor: the pitch's affordance for
 * "click me for this player's analysis". Drawn last so it sits on top.
 */
function drawHoveredPlayer() {
  if (!hoveredPlayer) return;
  const hp = lastPlayers.find(
    (p) => p.team === hoveredPlayer.team && p.shirt === hoveredPlayer.shirt,
  );
  if (!hp) {
    hoveredPlayer = null;
    return;
  }
  const [hx, hy] = toScreen(hp.x, hp.y);
  const entry = dribbleIndex?.playerByKey(hp.team, hp.shirt);
  const label = entry?.name ? `${entry.name} · #${hp.shirt}` : `#${hp.shirt}`;
  ctx.save();
  ctx.beginPath();
  ctx.arc(hx, hy, 10, 0, Math.PI * 2);
  ctx.strokeStyle = 'rgba(47,129,247,0.95)';
  ctx.lineWidth = 2;
  ctx.stroke();
  ctx.font = 'bold 11px ui-sans-serif, system-ui, sans-serif';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  const tw = ctx.measureText(label).width;
  const boxW = tw + 14;
  const boxH = 18;
  // Flip below the ring at the top edge, and clamp inside the canvas, so
  // hovering a body on the touchline never clips the label.
  const boxY = hy - 34 < 4 ? hy + 16 : hy - 34;
  const boxX = Math.max(2, Math.min(els.canvas.width - boxW - 2, hx - boxW / 2));
  ctx.fillStyle = 'rgba(13,17,23,0.94)';
  ctx.fillRect(boxX, boxY, boxW, boxH);
  ctx.strokeStyle = 'rgba(47,129,247,0.9)';
  ctx.lineWidth = 1;
  ctx.strokeRect(boxX, boxY, boxW, boxH);
  ctx.fillStyle = '#e6edf3';
  ctx.fillText(label, boxX + boxW / 2, boxY + boxH / 2 + 0.5);
  ctx.restore();
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

function updateStats() {
  if (!engine) return;
  els.score.textContent = `${engine.get_score_home()} – ${engine.get_score_away()}`;
  const ph = engine.get_possession_home();
  els.posshome.style.width = `${(ph * 100).toFixed(0)}%`;
  els.possaway.style.width = `${((1 - ph) * 100).toFixed(0)}%`;
  els.possval.textContent = `${(ph * 100).toFixed(0)}%`;
}

/** HUD match clock — shared by playback and seeks, so they never disagree. */
function updateClockLabel() {
  if (!engine) return;
  const t = engine.get_sim_time();
  els.timer.textContent = `${String(Math.floor(t / 60)).padStart(2, '0')}:${String(Math.floor(t % 60)).padStart(2, '0')}`;
}

// ── Momentum strip (sidebar canvas) ──────────────────────────
// Cumulative model-estimate threat per team; goals/shot/card ticks mark it.
let lastMomentumDraw = 0;
function drawMomentum() {
  const el = document.getElementById('momentum');
  if (!el) return;
  const now = performance.now();
  if (now - lastMomentumDraw < 500) return;
  lastMomentumDraw = now;
  const c = el.getContext('2d');
  const W = el.width;
  const H = el.height;
  c.clearRect(0, 0, W, H);
  const series = [momentum.home, momentum.away];
  const colors = ['#f2f2f2', '#2f81f7'];
  const all = series.flat();
  if (all.length < 2) {
    c.fillStyle = 'rgba(139,148,158,0.8)';
    c.font = '10px ui-sans-serif, system-ui, sans-serif';
    c.textAlign = 'center';
    c.fillText('momentum builds as the match plays', W / 2, H / 2);
    return;
  }
  const t0 = all[0].t;
  const t1 = Math.max(all[all.length - 1].t, t0 + 1);
  let vmax = 0.5;
  for (const s of series) for (const r of s) vmax = Math.max(vmax, r.v);
  const X = (t) => 6 + ((t - t0) / (t1 - t0)) * (W - 12);
  const Y = (v) => H - 8 - (v / vmax) * (H - 20);
  c.strokeStyle = 'rgba(48,54,61,0.8)';
  c.lineWidth = 1;
  c.beginPath();
  c.moveTo(0, Y(0));
  c.lineTo(W, Y(0));
  c.stroke();
  series.forEach((s, k) => {
    if (!s.length) return;
    c.strokeStyle = colors[k];
    c.lineWidth = 1.6;
    c.beginPath();
    c.moveTo(X(s[0].t), Y(s[0].v));
    for (const r of s) c.lineTo(X(r.t), Y(r.v));
    c.stroke();
    // Event ticks: goals big, shots/cards small, with bloom.
    for (const r of s) {
      const big = r.type === 16 || r.type === 17;
      const isShot = r.type === 13 || r.type === 14 || r.type === 15;
      const pulse = 1 + 0.35 * Math.sin((performance.now() / 1000) * 4 + r.t);
      // bloom
      if (big || isShot) {
        const bloomR = (big ? 7 : 4) * pulse;
        const g = c.createRadialGradient(X(r.t), Y(r.v), 0, X(r.t), Y(r.v), bloomR);
        g.addColorStop(0, big ? 'rgba(255,234,0,0.55)' : 'rgba(255,200,80,0.4)');
        g.addColorStop(1, 'rgba(0,0,0,0)');
        c.fillStyle = g;
        c.beginPath();
        c.arc(X(r.t), Y(r.v), bloomR, 0, Math.PI * 2);
        c.fill();
      }
      c.fillStyle = big ? colors[k] : (isShot ? 'rgba(255,200,80,0.9)' : 'rgba(139,148,158,0.9)');
      c.beginPath();
      c.arc(X(r.t), Y(r.v), (big ? 3 : 1.6) * pulse, 0, Math.PI * 2);
      c.fill();
    }
  });
}

// ── Main loop (fixed-step accumulator) ─────────────────────────
function loop(now) {
  try {
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

      updateClockLabel();
      updateEvents();
      updateStats();

      // Full time: stop advancing so the pitch freezes on a finished match.
      if (engine.get_is_finished()) {
        playing = false;
        els.playpause.textContent = '▶ Play';
        setStatus('Full time — all events played.');
      }
    }

    render(frame);
  } catch (e) {
    setStatus(`Frame error: ${e && e.message ? e.message : e}`);
  } finally {
    requestAnimationFrame(loop);
  }
}
for (const button of effectButtons) {
  button.addEventListener('click', () => {
    const key = button.dataset.effect;
    effects.toggleEnabled(key);
    syncEffectButtons();
    // Turning on a layer that has nothing to draw deserves an explanation.
    if (key === 'dribbles' && effects.isEnabled('dribbles') && dribbleIndex && !dribbleIndex.hasDribbleData) {
      setStatus('This feed records no take-on or carry events — dribble zones and stats stay empty.');
    }
  });
}
if (els.shuffle) {
  els.shuffle.addEventListener('click', () => {
    effects.toggleShuffle();
    syncEffectButtons();
  });
}

// ── Pitch pointer interaction ──────────────────────────────────
// Hover reads the body under the cursor; a click opens the player context
// menu (or filters the panel by zone); dragging pans; ctrl/cmd + wheel and
// the zoom slider share one view transform.

function hitTestPlayer(canvasX, canvasY) {
  if (!lastPlayers.length) return null;
  const [bx, by] = toBaseScreen(canvasX, canvasY);
  let best = null;
  let bestD = 11 * 11; // base-screen px: 5 px marker + slack
  for (const p of lastPlayers) {
    const [sx, sy] = toScreen(p.x, p.y);
    const d = (sx - bx) * (sx - bx) + (sy - by) * (sy - by);
    if (d < bestD) { bestD = d; best = p; }
  }
  return best;
}

function hitTestZone(canvasX, canvasY) {
  if (!dribblePanel || !effects.isVisible('dribbles')) return null;
  const [wx, wy] = fromScreen(canvasX, canvasY);
  if (Math.abs(wx) > PITCH_W_M / 2 || Math.abs(wy) > PITCH_H_M / 2) return null;
  return zoneOf(wx, wy, dribblePanel.layerTeam());
}

function hidePlayerMenu() {
  const menu = els.playerContext;
  if (!menu || menu.hidden) return;
  menu.hidden = true;
  menu.innerHTML = '';
}

/** Open the click menu for a pitch body, kept inside the stage. */
function showPlayerMenu(player, ev) {
  const menu = els.playerContext;
  if (!menu || !dribbleIndex) return;
  const entry = dribbleIndex.playerByKey(player.team, player.shirt);
  const pid = entry ? entry.pid : null;
  const dribbles = pid ? dribbleIndex.dribblesOf(pid) : [];

  menu.innerHTML = '';
  const title = document.createElement('div');
  title.className = 'ctx-title';
  const strong = document.createElement('strong');
  strong.textContent = entry ? entry.label : `#${player.shirt}`;
  title.appendChild(strong);
  title.appendChild(document.createTextNode(
    `${player.team === 0 ? 'Home' : 'Away'} · #${player.shirt}`,
  ));
  menu.appendChild(title);

  const addItem = (label, fn) => {
    const button = document.createElement('button');
    button.type = 'button';
    button.setAttribute('role', 'menuitem');
    button.textContent = label;
    button.addEventListener('click', () => { hidePlayerMenu(); fn(); });
    menu.appendChild(button);
  };

  addItem('Open player analysis', () => {
    if (pid) dribblePanel.open('player', { team: player.team, player: pid });
    else setStatus('That player is not in the match roster.');
  });
  addItem('Show team dribble zones', () => {
    effects.setEnabled('dribbles', true);
    syncEffectButtons();
    dribblePanel.open('team', { team: player.team });
  });
  if (dribbles.length) {
    const first = dribbles[0];
    addItem(`Jump to first dribble · ${formatClock(first.t)}`, () => {
      seekTo(first.mt, `Jumped to ${formatClock(first.t)} — paused to inspect.`);
      playing = false;
      els.playpause.textContent = '▶ Play';
    });
  }

  menu.hidden = false;
  const stageRect = els.stage.getBoundingClientRect();
  const mw = menu.offsetWidth || 214;
  const mh = menu.offsetHeight || 140;
  let x = ev.clientX - stageRect.left;
  let y = ev.clientY - stageRect.top + 10;
  if (x + mw > stageRect.width) x = Math.max(4, x - mw - 14);
  if (y + mh > stageRect.height) y = Math.max(4, stageRect.height - mh - 6);
  menu.style.left = `${x}px`;
  menu.style.top = `${y}px`;
  menu.querySelector('button')?.focus();
}

els.canvas.addEventListener('pointerdown', (ev) => {
  if (ev.pointerType === 'mouse' && ev.button !== 0) return;
  panning = {
    x: ev.clientX, y: ev.clientY,
    panX: view.panX, panY: view.panY, moved: false,
  };
  try { els.canvas.setPointerCapture(ev.pointerId); } catch { /* ignore */ }
});

els.canvas.addEventListener('pointermove', (ev) => {
  if (panning) {
    const dx = ev.clientX - panning.x;
    const dy = ev.clientY - panning.y;
    if (Math.abs(dx) + Math.abs(dy) > 4) panning.moved = true;
    if (panning.moved) {
      const rect = els.canvas.getBoundingClientRect();
      const scale = els.canvas.width / rect.width;
      view.panX = panning.panX + dx * scale;
      view.panY = panning.panY + dy * scale;
      clampView();
    }
    return;
  }
  const [cx, cy] = pointerToCanvas(ev);
  const player = hitTestPlayer(cx, cy);
  const key = player ? `${player.team}:${player.shirt}` : '';
  const prevKey = hoveredPlayer ? `${hoveredPlayer.team}:${hoveredPlayer.shirt}` : '';
  if (key !== prevKey) {
    hoveredPlayer = player;
    els.canvas.style.cursor = player ? 'pointer' : 'default';
  }
  hoverZone = player ? null : hitTestZone(cx, cy);
});

els.canvas.addEventListener('pointerup', (ev) => {
  if (!panning) return;
  const moved = panning.moved;
  panning = null;
  if (moved) return; // a drag, not a click
  const [cx, cy] = pointerToCanvas(ev);
  const player = hitTestPlayer(cx, cy);
  if (player) {
    showPlayerMenu(player, ev);
    return;
  }
  hidePlayerMenu();
  const zone = hitTestZone(cx, cy);
  if (zone != null && dribblePanel) {
    // Clicking a zone selects it: open the team lens if the panel is shut.
    if (!dribblePanel.isOpen()) dribblePanel.open('team', { team: dribblePanel.layerTeam() });
    dribblePanel.setZone(dribblePanel.selectedZone === zone ? null : zone);
  }
});

els.canvas.addEventListener('pointercancel', () => { panning = null; });
els.canvas.addEventListener('pointerleave', () => {
  if (panning) return;
  hoveredPlayer = null;
  hoverZone = null;
  els.canvas.style.cursor = 'default';
});

// Ctrl/Cmd + wheel zooms around the cursor; a plain wheel keeps scrolling
// the page (the zoom slider is the discoverable control).
els.canvas.addEventListener('wheel', (ev) => {
  if (!ev.ctrlKey && !ev.metaKey) return;
  ev.preventDefault();
  const [cx, cy] = pointerToCanvas(ev);
  const z0 = view.zoom;
  const z1 = Math.min(VIEW_ZOOM_MAX, Math.max(1, z0 * (ev.deltaY < 0 ? 1.12 : 1 / 1.12)));
  if (z1 === z0) return;
  // Hold the point under the cursor still while zooming.
  const bx = (cx - CX - view.panX) / z0 + CX;
  const by = (cy - CY - view.panY) / z0 + CY;
  view.zoom = z1;
  view.panX = cx - CX - z1 * (bx - CX);
  view.panY = cy - CY - z1 * (by - CY);
  clampView();
  syncZoomControl();
}, { passive: false });

function syncZoomControl() {
  if (els.zoom) els.zoom.value = String(view.zoom);
  if (els.zoomval) els.zoomval.textContent = `${view.zoom.toFixed(2)}×`;
}

if (els.zoom) {
  els.zoom.addEventListener('input', () => {
    view.zoom = parseFloat(els.zoom.value) || 1;
    clampView();
    syncZoomControl();
  });
}
if (els.resetView) {
  els.resetView.addEventListener('click', () => {
    view.zoom = 1;
    view.panX = 0;
    view.panY = 0;
    syncZoomControl();
    setStatus('View reset.');
  });
}
if (els.zoneAlpha) {
  els.zoneAlpha.addEventListener('input', () => {
    zoneAlpha = Math.min(1, Math.max(0.15, parseFloat(els.zoneAlpha.value) || 0.55));
    if (els.zoneAlphaVal) els.zoneAlphaVal.textContent = `${Math.round(zoneAlpha * 100)}%`;
  });
}
syncZoomControl();

// Click-away and Escape dismiss the context menu (the panel hides itself
// through its own keydown handler when focus is inside it).
document.addEventListener('pointerdown', (ev) => {
  const menu = els.playerContext;
  if (!menu || menu.hidden) return;
  if (menu.contains(ev.target) || ev.target === els.canvas) return;
  hidePlayerMenu();
});
document.addEventListener('keydown', (ev) => {
  if (ev.key === 'Escape') hidePlayerMenu();
});
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
els.back5.addEventListener('click', () => seekBy(-300));
els.fwd5.addEventListener('click', () => seekBy(300));
els.browse.addEventListener('click', browseDirectory);

function seekBy(seconds) {
  if (!engine) return;
  seekTo(engine.get_sim_time() + seconds);
}

/**
 * Absolute seek on the engine's match clock (the rebased chronology the
 * analysis log's `match_secs` column shares). Resets every transient
 * overlay so a jump never replays stale choreography.
 */
function seekTo(target, message) {
  if (!engine) return;
  const dur = engine.get_duration_secs();
  const clamped = Math.max(0, Math.min(dur, target));
  engine.seek_to(clamped);
  lastEventSeq = -1;
  resetSequenceCursor();
  resetOverlayState();
  halftimeActive = false;
  halftimeStartPositions = [];
  halftimeKickoffReady = false;
  lastPeriod = typeof engine.get_current_period === 'function' ? engine.get_current_period() : 1;
  if (els.halftimeBanner) els.halftimeBanner.classList.remove('show');
  substitutions.reset();
  celebrations.reset();
  fx.reset();
  momentum.reset();
  passingNet.reset();
  passingLanes.reset();
  attackChannelsFx.reset();
  playerHeat.reset();
  chosenPlayerShirt = null;
  playerHeatTimer = 0;
  lastPlayerHeatTeam = -1;
  lastPlayerHeatSample = -Infinity;
  lastLaneCompute = -Infinity;
  lastLaneData = [];
  lastChannelCompute = -Infinity;
  lastChannelData = [];
  playerVelocity.clear();
  visionAlpha = 0;
  lastVisionCarrier = '';
  lastVisionCone = null;
  visionFocus = null;
  if (highlighter) { highlighter.buffer.length = 0; }
  lastPasser[0] = { id: '', pos: null };
  lastPasser[1] = { id: '', pos: null };
  els.feed.innerHTML = '';
  setStatus(message ?? `Jumped to ${Math.floor(clamped / 60)} min`);
  // A seek rebuilds the engine; the HUD must show the new point immediately
  // (it used to wait for playback to resume, leaving the clock stale).
  updateClockLabel();
  updateStats();
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

// Read-only introspection handle for automated checks and console
// debugging: exposes live references, never mutates the sim itself.
window.__gev = {
  get engine() { return engine; },
  get players() { return lastPlayers; },
  get panel() { return dribblePanel; },
  get index() { return dribbleIndex; },
  get view() { return view; },
  toScreen,
  seekTo,
};

main();
