// Canvas drawing for the team overlays: heatmap, offside line, broadcast analysis.

import { smoothstep } from './easing.js';
import { PITCH } from './analysis.js';

// Screen-space hit targets for the pass-lane destination chips. The app reads
// this after each frame so a chip can open the receiver's player panel.
export const laneTargetHits = [];

const HOME_HEAT = [
  [0.0, 0, 0, 0, 0],
  [0.35, 255, 236, 170, 13],
  [0.65, 255, 214, 120, 26],
  [1.0, 255, 190, 90, 46],
];
const AWAY_HEAT = [
  [0.0, 0, 0, 0, 0],
  [0.35, 190, 240, 255, 13],
  [0.65, 140, 200, 255, 26],
  [1.0, 110, 160, 255, 46],
];

function buildHeatLut(stops) {
  const lut = new Uint8ClampedArray(256 * 4);
  for (let i = 0; i < 256; i++) {
    const t = i / 255;
    let a = stops[0];
    let b = stops[stops.length - 1];
    for (let s = 0; s < stops.length - 1; s++) {
      if (t >= stops[s][0] && t <= stops[s + 1][0]) {
        a = stops[s];
        b = stops[s + 1];
        break;
      }
    }
    const span = b[0] - a[0] || 1;
    const f = Math.min(1, Math.max(0, (t - a[0]) / span));
    lut[i * 4] = a[1] + (b[1] - a[1]) * f;
    lut[i * 4 + 1] = a[2] + (b[2] - a[2]) * f;
    lut[i * 4 + 2] = a[3] + (b[3] - a[3]) * f;
    lut[i * 4 + 3] = a[4] + (b[4] - a[4]) * f;
  }
  return lut;
}

const HEAT_LUTS = [buildHeatLut(HOME_HEAT), buildHeatLut(AWAY_HEAT)];
const heatCaches = [
  { canvas: null, ctx: null, version: -1, cols: 0, rows: 0,
    hexCanvas: null, hexCtx: null, hexVersion: -1, hexW: 0, hexH: 0 },
  { canvas: null, ctx: null, version: -1, cols: 0, rows: 0,
    hexCanvas: null, hexCtx: null, hexVersion: -1, hexW: 0, hexH: 0 },
];

export function drawHeatmap(ctx, rect, heat, team, alpha = 1, variant = 'raster') {
  if (!heat || heat.max <= 0 || alpha <= 0.01) return;
  const slot = team === 1 ? 1 : 0;
  const cache = heatCaches[slot];
  if (variant === 'hexbin') {
    const w = Math.max(1, Math.round(rect.w));
    const h = Math.max(1, Math.round(rect.h));
    if (!cache.hexCanvas) {
      cache.hexCanvas = document.createElement('canvas');
      cache.hexCtx = cache.hexCanvas.getContext('2d');
    }
    if (cache.hexW !== w || cache.hexH !== h) {
      cache.hexCanvas.width = w;
      cache.hexCanvas.height = h;
      cache.hexW = w;
      cache.hexH = h;
      cache.hexVersion = -1;
    }
    if (cache.hexVersion !== heat.version) {
      cache.hexVersion = heat.version;
      const lut = HEAT_LUTS[slot];
      const invMax = 1 / heat.max;
      const cw = w / heat.cols;
      const ch = h / heat.rows;
      const r = Math.min(cw, ch) * 0.58;
      const hctx = cache.hexCtx;
      hctx.clearRect(0, 0, w, h);
      for (let y = 0; y < heat.rows; y++) {
        for (let x = 0; x < heat.cols; x++) {
          const v = heat.grid[y * heat.cols + x];
          if (v <= 0) continue;
          const t = Math.pow(v * invMax, 0.78);
          const li = Math.min(255, Math.round(t * 255)) * 4;
          const cx = (x + 0.5) * cw;
          const cy = (y + 0.5) * ch;
          hctx.fillStyle = `rgba(${lut[li]},${lut[li+1]},${lut[li+2]},${lut[li+3]/255})`;
          hctx.beginPath();
          for (let i = 0; i < 6; i++) {
            const a = Math.PI / 3 * i;
            const px = cx + r * Math.cos(a);
            const py = cy + r * Math.sin(a);
            if (i === 0) hctx.moveTo(px, py); else hctx.lineTo(px, py);
          }
          hctx.closePath();
          hctx.fill();
        }
      }
    }
    ctx.save();
    ctx.imageSmoothingEnabled = true;
    ctx.globalCompositeOperation = 'screen';
    ctx.globalAlpha = alpha;
    ctx.drawImage(cache.hexCanvas, rect.x, rect.y, rect.w, rect.h);
    ctx.restore();
    return;
  }
  if (!cache.canvas) {
    cache.canvas = document.createElement('canvas');
    cache.ctx = cache.canvas.getContext('2d');
  }
  if (cache.canvas.width !== heat.cols || cache.canvas.height !== heat.rows) {
    cache.canvas.width = heat.cols;
    cache.canvas.height = heat.rows;
    cache.version = -1;
  }
  if (cache.version !== heat.version) {
    cache.version = heat.version;
    const img = cache.ctx.createImageData(heat.cols, heat.rows);
    const lut = HEAT_LUTS[slot];
    const invMax = 1 / heat.max;
    for (let i = 0; i < heat.grid.length; i++) {
      const t = Math.pow(heat.grid[i] * invMax, 0.78);
      const li = Math.min(255, Math.round(t * 255)) * 4;
      const o = i * 4;
      img.data[o] = lut[li];
      img.data[o + 1] = lut[li + 1];
      img.data[o + 2] = lut[li + 2];
      img.data[o + 3] = lut[li + 3];
    }
    cache.ctx.putImageData(img, 0, 0);
  }
  ctx.save();
  ctx.imageSmoothingEnabled = true;
  ctx.globalCompositeOperation = 'screen';
  ctx.globalAlpha = alpha;
  ctx.drawImage(cache.canvas, rect.x, rect.y, rect.w, rect.h);
  ctx.restore();
}

export function drawOffsideLine(ctx, toScreen, x, alpha, top, bottom) {
  if (!Number.isFinite(x) || alpha <= 0.01) return;
  const [sx] = toScreen(x, 0);
  ctx.save();
  ctx.setLineDash([7, 7]);
  ctx.strokeStyle = `rgba(255, 100, 100, ${(0.55 * alpha).toFixed(3)})`;
  ctx.lineWidth = 1.2;
  ctx.beginPath();
  ctx.moveTo(sx, top);
  ctx.lineTo(sx, bottom);
  ctx.stroke();
  ctx.restore();
}

function convexHull(points) {
  if (points.length < 3) return points.slice();
  const pts = points.slice().sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const cross = (o, a, b) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const lower = [];
  for (const p of pts) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) lower.pop();
    lower.push(p);
  }
  const upper = [];
  for (let i = pts.length - 1; i >= 0; i--) {
    const p = pts[i];
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) upper.pop();
    upper.push(p);
  }
  lower.pop();
  upper.pop();
  return lower.concat(upper);
}

export function drawAnalysis(ctx, toScreen, players, team, alpha, ball, possessionTeam, options = {}, variant = 'classic') {
  const showLinks = options?.links !== false;
  if (!players || players.length === 0 || alpha <= 0.01 || !ball) return;
  const teamPlayers = players.filter((p) => p.team === team);
  if (teamPlayers.length === 0) return;
  const teamRgb = team === 1 ? '47,129,247' : '242,242,242';
  const attackDir = team === 0 ? 1 : -1;

  ctx.save();

  let carrier = null;
  let bestD = Infinity;
  for (const p of teamPlayers) {
    const d = Math.hypot(p.x - ball.x, p.y - ball.y);
    if (d < bestD) { bestD = d; carrier = p; }
  }
  const hasCarrier = carrier && bestD <= 8;

  if (hasCarrier) {
    const [cx, cy] = toScreen(carrier.x, carrier.y);

    ctx.beginPath();
    ctx.arc(cx, cy, 7, 0, Math.PI * 2);
    ctx.strokeStyle = `rgba(${teamRgb},${(0.7 * alpha).toFixed(3)})`;
    ctx.lineWidth = 1.3;
    ctx.stroke();

    const teammates = teamPlayers
      .filter((p) => p !== carrier)
      .sort((a, b) =>
        Math.hypot(a.x - carrier.x, a.y - carrier.y) -
        Math.hypot(b.x - carrier.x, b.y - carrier.y));

    if (showLinks && teammates.length > 0) {
      const [rx, ry] = toScreen(teammates[0].x, teammates[0].y);
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.lineTo(rx, ry);
      ctx.setLineDash([4, 4]);
      ctx.strokeStyle = `rgba(${teamRgb},${(0.45 * alpha).toFixed(3)})`;
      ctx.lineWidth = 1.2;
      ctx.stroke();
      ctx.setLineDash([]);
    }

    if (showLinks && teammates.length >= 2) {
      const a = toScreen(carrier.x, carrier.y);
      const b = toScreen(teammates[0].x, teammates[0].y);
      const c = toScreen(teammates[1].x, teammates[1].y);
      ctx.beginPath();
      ctx.moveTo(a[0], a[1]);
      ctx.lineTo(b[0], b[1]);
      ctx.lineTo(c[0], c[1]);
      ctx.closePath();
      ctx.strokeStyle = `rgba(${teamRgb},${(0.35 * alpha).toFixed(3)})`;
      ctx.lineWidth = 1;
      ctx.stroke();
    }

    const progress = carrier.x * attackDir;
    if (progress > 20) {
      const goalX = attackDir * 52.5;
      const [gx, gy] = toScreen(goalX, 0);
      const len = Math.hypot(gx - cx, gy - cy) || 1;
      const ux = (gx - cx) / len;
      const uy = (gy - cy) / len;
      const tipX = cx + ux * 40;
      const tipY = cy + uy * 40;
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.lineTo(tipX, tipY);
      ctx.strokeStyle = `rgba(${teamRgb},${(0.25 * alpha).toFixed(3)})`;
      ctx.lineWidth = 1;
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(tipX, tipY);
      ctx.lineTo(tipX - uy * 6 - ux * 6, tipY + ux * 6 - uy * 6);
      ctx.lineTo(tipX + uy * 6 - ux * 6, tipY - ux * 6 - uy * 6);
      ctx.closePath();
      ctx.fillStyle = `rgba(${teamRgb},${(0.3 * alpha).toFixed(3)})`;
      ctx.fill();

      if (variant !== 'minimal') {
      ctx.save();
      ctx.font = '700 9px ui-sans-serif, system-ui, sans-serif';
      ctx.textAlign = 'left';
      ctx.textBaseline = 'middle';
      ctx.lineWidth = 3;
      ctx.strokeStyle = 'rgba(13,17,23,0.85)';
      ctx.strokeText('THREAT', cx + 12, cy - 12);
      ctx.fillStyle = `rgba(255,180,80,${(0.9 * alpha).toFixed(3)})`;
      ctx.fillText('THREAT', cx + 12, cy - 12);
      ctx.restore();
      }

      if (teammates.length >= 2) {
        const opponents = players.filter((p) => p.team !== team);
        let presser = null;
        let pressD = Infinity;
        for (const p of opponents) {
          const d = Math.hypot(p.x - carrier.x, p.y - carrier.y);
          if (d < pressD) { pressD = d; presser = p; }
        }
        if (presser) {
          const pts = [
            toScreen(carrier.x, carrier.y),
            toScreen(teammates[0].x, teammates[0].y),
            toScreen(teammates[1].x, teammates[1].y),
            toScreen(presser.x, presser.y),
          ];
          ctx.beginPath();
          ctx.moveTo(pts[0][0], pts[0][1]);
          for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i][0], pts[i][1]);
          ctx.closePath();
          ctx.fillStyle = `rgba(255,180,80,${(0.06 * alpha).toFixed(3)})`;
          ctx.fill();
          ctx.strokeStyle = `rgba(255,180,80,${(0.4 * alpha).toFixed(3)})`;
          ctx.lineWidth = 1;
          ctx.setLineDash([3, 3]);
          ctx.stroke();
          ctx.setLineDash([]);
        }
      }
    }

    const opponents2 = players.filter((p) => p.team !== team);
    let presser2 = null;
    let pressD2 = Infinity;
    for (const p of opponents2) {
      const d = Math.hypot(p.x - carrier.x, p.y - carrier.y);
      if (d < pressD2) { pressD2 = d; presser2 = p; }
    }
    if (variant !== 'minimal' && presser2 && pressD2 < 15) {
      const [px, py] = toScreen(presser2.x, presser2.y);
      ctx.beginPath();
      ctx.arc(px, py, 6, 0, Math.PI * 2);
      ctx.strokeStyle = `rgba(255,100,100,${(0.5 * alpha).toFixed(3)})`;
      ctx.lineWidth = 1.1;
      ctx.setLineDash([3, 3]);
      ctx.stroke();
      ctx.setLineDash([]);
    }
  }

  const isPossession = possessionTeam === team;
  if (!isPossession) {
    const defenders = teamPlayers.filter((p) => p.role === 2 || p.role === 0);
    let hullPlayers = defenders;
    if (hullPlayers.length < 3) {
      hullPlayers = teamPlayers
        .slice()
        .sort((a, b) => a.x * attackDir - b.x * attackDir)
        .slice(0, Math.min(4, teamPlayers.length));
    }
    if (hullPlayers.length >= 3) {
      const pts = hullPlayers.map((p) => [p.x, p.y]);
      const hull = convexHull(pts);
      if (hull.length >= 3) {
        ctx.beginPath();
        const [sx0, sy0] = toScreen(hull[0][0], hull[0][1]);
        ctx.moveTo(sx0, sy0);
        for (let i = 1; i < hull.length; i++) {
          const [sx, sy] = toScreen(hull[i][0], hull[i][1]);
          ctx.lineTo(sx, sy);
        }
        ctx.closePath();
        ctx.fillStyle = `rgba(${teamRgb},0.04)`;
        ctx.fill();
        ctx.strokeStyle = `rgba(${teamRgb},${(0.3 * alpha).toFixed(3)})`;
        ctx.lineWidth = 1;
        ctx.stroke();
      }
    }
  }

  ctx.restore();
}

// ── Frozen-frame comparison ────────────────────────────────────
/**
 * Ghost of a captured instant: dashed bodies, a dashed ball and a faint
 * team-coloured fill. The live match keeps running underneath, so the
 * analyst can see what changed since the freeze.
 */
export function drawFrozenFrame(ctx, toScreen, frame, outerAlpha = 1) {
  if (!frame || outerAlpha <= 0.01) return;
  // A previous-possession snapshot is drawn first, faintly, so the frozen
  // frame answers "what changed since the ball turned over?".
  if (frame.previous) drawFrozenFrame(ctx, toScreen, frame.previous, outerAlpha * 0.32);
  ctx.save();
  // Frozen analytical context: the sight picture and lane geometry at the
  // instant of capture, drawn beneath the body ghosts.
  if (frame.vision?.conePoints?.length >= 3) {
    clipPitch(ctx, toScreen);
    if (traceWorldPolygon(ctx, toScreen, frame.vision.conePoints)) {
      ctx.globalAlpha = 0.10 * outerAlpha;
      ctx.fillStyle = 'rgba(80,220,180,1)';
      ctx.fill();
      ctx.globalAlpha = 0.42 * outerAlpha;
      ctx.strokeStyle = 'rgba(120,255,220,0.95)';
      ctx.lineWidth = 1;
      ctx.setLineDash([4, 5]);
      ctx.stroke();
      ctx.setLineDash([]);
    }
  }
  for (const lane of frame.lanes ?? []) {
    if (!Number.isFinite(lane.centerY) || !Number.isFinite(lane.x0) || !Number.isFinite(lane.x1)) continue;
    const [x0, y0] = toScreen(lane.x0, lane.centerY);
    const [x1, y1] = toScreen(lane.x1, lane.centerY);
    const [, top] = toScreen(lane.x0, lane.y1);
    const [, bottom] = toScreen(lane.x0, lane.y0);
    ctx.globalAlpha = 0.055 * outerAlpha;
    ctx.fillStyle = 'rgba(120,220,255,1)';
    ctx.fillRect(Math.min(x0, x1), Math.min(top, bottom), Math.abs(x1 - x0), Math.abs(bottom - top));
    ctx.globalAlpha = 0.34 * outerAlpha;
    ctx.strokeStyle = 'rgba(150,235,255,0.95)';
    ctx.lineWidth = 1;
    ctx.setLineDash([5, 6]);
    ctx.beginPath();
    ctx.moveTo(x0, y0);
    ctx.lineTo(x1, y1);
    ctx.stroke();
  }
  ctx.setLineDash([]);
  for (const p of frame.players ?? []) {
    const [sx, sy] = toScreen(p.x, p.y);
    const rgb = p.team === 1 ? '47,129,247' : '242,242,242';
    ctx.globalAlpha = 0.16 * outerAlpha;
    ctx.fillStyle = `rgba(${rgb},1)`;
    ctx.beginPath();
    ctx.arc(sx, sy, 5.5, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 0.55 * outerAlpha;
    ctx.strokeStyle = `rgba(${rgb},1)`;
    ctx.lineWidth = 1.1;
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    ctx.arc(sx, sy, 5.5, 0, Math.PI * 2);
    ctx.stroke();
    if (Number.isFinite(p.shirt)) {
      ctx.globalAlpha = 0.72 * outerAlpha;
      ctx.fillStyle = 'rgba(230,237,243,1)';
      ctx.font = '700 6.5px ui-sans-serif, system-ui, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(String(p.shirt), sx, sy + 0.3);
    }
  }
  if (frame.ball) {
    const [bx, by] = toScreen(frame.ball.x, frame.ball.y);
    ctx.globalAlpha = 0.7 * outerAlpha;
    ctx.strokeStyle = 'rgba(255,234,0,1)';
    ctx.lineWidth = 1.2;
    ctx.setLineDash([2, 3]);
    ctx.beginPath();
    ctx.arc(bx, by, 6.5, 0, Math.PI * 2);
    ctx.stroke();
  }
  ctx.setLineDash([]);
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Substitution ghosts ────────────────────────────────────────
export function drawSubstitutionFx(ctx, toScreen, items) {
  if (!items || items.length === 0) return;
  ctx.save();
  for (const t of items) {
    if (t.outgoing) {
      const [sx, sy] = toScreen(t.outgoing.x, t.outgoing.y);
      ctx.globalAlpha = Math.max(0, Math.min(1, t.outgoing.alpha ?? 0.5));
      ctx.beginPath();
      ctx.arc(sx, sy, 5.5, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(255,200,120,0.7)';
      ctx.fill();
      ctx.strokeStyle = 'rgba(13,17,23,0.9)';
      ctx.lineWidth = 1;
      ctx.stroke();
      ctx.fillStyle = 'rgba(13,17,23,0.9)';
      ctx.font = '700 6px ui-sans-serif, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(String(t.outgoing.shirt), sx, sy + 0.2);
    }
    if (t.incoming) {
      const [sx, sy] = toScreen(t.incoming.x, t.incoming.y);
      ctx.globalAlpha = Math.max(0, Math.min(1, t.incoming.alpha ?? 0.5));
      ctx.beginPath();
      ctx.arc(sx, sy, 5.5, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(120,220,160,0.7)';
      ctx.fill();
      ctx.strokeStyle = 'rgba(13,17,23,0.9)';
      ctx.lineWidth = 1;
      ctx.stroke();
      ctx.fillStyle = 'rgba(13,17,23,0.9)';
      ctx.font = '700 6px ui-sans-serif, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(String(t.incoming.shirt), sx, sy + 0.2);
    }
  }
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Celebration ghosts ─────────────────────────────────────────
export function drawCelebrationFx(ctx, toScreen, fx) {
  if (!fx || (!fx.run && (!fx.mob || fx.mob.length === 0))) return;
  ctx.save();
  if (fx.run) {
    const [sx, sy] = toScreen(fx.run.x, fx.run.y);
    ctx.globalAlpha = Math.max(0, Math.min(1, fx.run.alpha ?? 0.5));
    ctx.beginPath();
    ctx.arc(sx, sy, 7, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(255,215,130,0.9)';
    ctx.fill();
    ctx.strokeStyle = 'rgba(13,17,23,0.9)';
    ctx.lineWidth = 1.1;
    ctx.stroke();
    ctx.fillStyle = 'rgba(13,17,23,0.9)';
    ctx.font = '700 7px ui-sans-serif, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(String(fx.run.shirt), sx, sy + 0.2);
  }
  for (const m of (fx.mob || [])) {
    const [sx, sy] = toScreen(m.x, m.y);
    ctx.globalAlpha = Math.max(0, Math.min(1, m.alpha ?? 0.5));
    ctx.beginPath();
    ctx.arc(sx, sy, 4.5, 0, Math.PI * 2);
    ctx.fillStyle = 'rgba(255,215,130,0.6)';
    ctx.fill();
  }
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Attack lanes (four horizontal channels on the attacked half) ──
/**
 * Draw the possession team's four attacking channels. Each lane is a faint
 * band with a glowing centre line; brightness comes from the carrier's
 * projected momentum plus the best viable pass into that band. Lanes grow
 * from the halfway line toward the goal and carry a bright pulse after a
 * completed pass. No arrowheads: the lane itself is the message.
 */
export function drawAttackLanes(ctx, toScreen, lanes, outerAlpha = 1, variant = 'classic', activeTeam = null, resolveTarget = null) {
  laneTargetHits.length = 0;
  if (!lanes || lanes.length === 0 || outerAlpha <= 0.01) return;
  const visible = lanes.filter((lane) => (lane.alpha ?? 1) > 0.01);
  if (visible.length === 0) return;
  ctx.save();
  clipPitch(ctx, toScreen);
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  // Weakest first: the projected lane's glow stays on top.
  const ordered = [...visible].sort((a, b) => (a.score ?? 0) - (b.score ?? 0));
  for (const lane of ordered) {
    const ghost = Number.isFinite(activeTeam) && Number.isFinite(lane.team) && lane.team !== activeTeam;
    const a = (lane.alpha ?? 1) * outerAlpha * (ghost ? 0.32 : 1);
    if (a <= 0.01) continue;
    if (lane.blur > 0.05) {
      try { ctx.filter = `blur(${lane.blur.toFixed(2)}px)`; } catch { /* ignore */ }
    } else {
      ctx.filter = 'none';
    }
    const score = Math.min(1, Math.max(0, lane.score ?? 0.5));
    const growth = Math.min(1, Math.max(0, lane.growth ?? 1));
    const y = lane.centerY;
    const tipX = lane.x0 + (lane.x1 - lane.x0) * growth;
    const [sx0, sy] = toScreen(lane.x0, y);
    const [sxTip, syTip] = toScreen(tipX, y);

    // 1. Directional band wash: quiet at halfway, brighter toward the goal.
    const [bx0, byTop] = toScreen(lane.x0, lane.y1);
    const [bx1, byBottom] = toScreen(tipX, lane.y0);
    const band = ctx.createLinearGradient(sx0, sy, sxTip, syTip);
    band.addColorStop(0, 'rgba(120,220,255,0)');
    band.addColorStop(0.45, 'rgba(120,220,255,0.55)');
    band.addColorStop(1, 'rgba(170,245,255,1)');
    ctx.globalAlpha = a * (0.035 + 0.085 * score);
    ctx.fillStyle = band;
    ctx.fillRect(
      Math.min(bx0, bx1),
      Math.min(byTop, byBottom),
      Math.abs(bx1 - bx0),
      Math.abs(byBottom - byTop),
    );

    // 2. Internal channel boundaries: dashed, so they never read as pitch
    // markings. Touchline edges are left to the pitch itself.
    ctx.globalAlpha = a * (0.03 + 0.05 * score);
    ctx.strokeStyle = 'rgba(120,220,255,1)';
    ctx.lineWidth = 1;
    ctx.setLineDash([10, 12]);
    for (const edgeY of [lane.y0, lane.y1]) {
      if (Math.abs(edgeY) >= PITCH.halfWidth - 0.01) continue;
      const [ex0, ey0] = toScreen(lane.x0, edgeY);
      const [ex1, ey1] = toScreen(tipX, edgeY);
      ctx.beginPath();
      ctx.moveTo(ex0, ey0);
      ctx.lineTo(ex1, ey1);
      ctx.stroke();
    }
    ctx.setLineDash([]);

    // 3. Glow under-stroke, then the bright core. Both fade in from halfway
    // so the channel grows out of the line rather than starting abruptly.
    const line = ctx.createLinearGradient(sx0, sy, sxTip, syTip);
    line.addColorStop(0, 'rgba(120,220,255,0.12)');
    line.addColorStop(0.35, 'rgba(120,220,255,0.8)');
    line.addColorStop(1, 'rgba(190,250,255,1)');
    ctx.strokeStyle = line;
    ctx.globalAlpha = a * (0.06 + 0.17 * score);
    ctx.lineWidth = 5 + 5 * score;
    ctx.beginPath();
    ctx.moveTo(sx0, sy);
    ctx.lineTo(sxTip, syTip);
    ctx.stroke();
    ctx.globalAlpha = a * (0.12 + 0.5 * score);
    ctx.lineWidth = 1.2 + 2.0 * score;
    ctx.beginPath();
    ctx.moveTo(sx0, sy);
    ctx.lineTo(sxTip, syTip);
    ctx.stroke();

    // 3a. Ghost identity stripe: a dim lane from the previous possession
    // keeps its team colour so the handover is readable at a glance.
    if (ghost) {
      ctx.globalAlpha = a * 0.55;
      ctx.strokeStyle = lane.team === 1 ? 'rgba(47,129,247,1)' : 'rgba(242,242,242,1)';
      ctx.lineWidth = 1.6;
      ctx.beginPath();
      ctx.moveTo(sx0, sy - 3);
      ctx.lineTo(sxTip, sy - 3);
      ctx.stroke();
    }

    // 3b. Ghost trail: three fading echoes behind the leading edge, so the
    // lane reads as a moving corridor rather than a static bar.
    if (variant === 'ghost-trail') {
      const dirSign = Math.sign(sxTip - sx0) || 1;
      for (let k = 1; k <= 3; k++) {
        const off = dirSign * k * 7;
        ctx.globalAlpha = a * (0.18 - k * 0.04) * (0.5 + score);
        ctx.lineWidth = 1.0;
        ctx.beginPath();
        ctx.moveTo(sx0, sy);
        ctx.lineTo(sxTip - off, syTip);
        ctx.stroke();
      }
    }

    // 4. Soft leading cap: the lane arrives rather than stopping dead.
    const headR = 5 + 8 * score;
    const head = ctx.createRadialGradient(sxTip, syTip, 0, sxTip, syTip, headR);
    head.addColorStop(0, 'rgba(170,240,255,0.55)');
    head.addColorStop(1, 'rgba(120,220,255,0)');
    ctx.globalAlpha = a * (0.12 + 0.22 * score);
    ctx.fillStyle = head;
    ctx.beginPath();
    ctx.arc(sxTip, syTip, headR, 0, Math.PI * 2);
    ctx.fill();

    // 4b. Corridor variant: name the pass destination at the end of the lane
    // instead of animating an unexplained glowing dot.
    if (variant === 'corridor' && lane.target) {
      const live = resolveTarget ? resolveTarget(lane.target) : null;
      const targetX = Number.isFinite(live?.x) ? live.x : lane.target.x;
      const targetY = Number.isFinite(live?.y) ? live.y : lane.target.y;
      const [tx, ty] = toScreen(targetX, targetY);
      const [px, py] = toScreen(lane.target.x, lane.target.y);
      if (Math.hypot(tx - px, ty - py) > 3) {
        ctx.globalAlpha = a * 0.18;
        ctx.strokeStyle = 'rgba(170,245,255,0.8)';
        ctx.lineWidth = 1;
        ctx.setLineDash([2, 4]);
        ctx.beginPath();
        ctx.moveTo(px, py);
        ctx.lineTo(tx, ty);
        ctx.stroke();
        ctx.setLineDash([]);
      }
      ctx.globalAlpha = a * 0.9;
      ctx.fillStyle = 'rgba(10, 20, 28, 0.82)';
      ctx.strokeStyle = 'rgba(170,245,255,0.9)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(tx, ty, 8, 0, Math.PI * 2);
      ctx.fill();
      ctx.stroke();
      if (Number.isFinite(lane.target.shirt)) {
        ctx.fillStyle = 'rgba(220,255,255,1)';
        ctx.font = '700 8px ui-sans-serif, system-ui, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(String(lane.target.shirt), tx, ty - 2);
        if (Number.isFinite(lane.target.p)) {
          ctx.fillStyle = 'rgba(160,235,255,0.92)';
          ctx.font = '600 6.5px ui-sans-serif, system-ui, sans-serif';
          ctx.fillText(`${Math.round(lane.target.p * 100)}%`, tx, ty + 6);
        }
        laneTargetHits.push({
          x: tx,
          y: ty,
          shirt: lane.target.shirt,
          team: lane.target.team,
          p: lane.target.p,
        });
      }
    }

    // 5. Projection marker: where the carrier's velocity and acceleration put
    // him in ~1.8 s. A scan line plus a soft bead makes the brightest lane
    // answer "where is he going next?", not just "which lane is he in?".
    const pp = lane.projected ? lane.projectedPoint : null;
    if (pp) {
      const minX = Math.min(lane.x0, lane.x1);
      const maxX = Math.max(lane.x0, lane.x1);
      if (pp.x >= minX && pp.x <= maxX) {
        const [mx, my] = toScreen(pp.x, pp.y);
        // Scan line across the band.
        const [, vy0] = toScreen(pp.x, lane.y0);
        const [, vy1] = toScreen(pp.x, lane.y1);
        ctx.globalAlpha = a * (0.08 + 0.12 * score);
        ctx.strokeStyle = 'rgba(190,250,255,1)';
        ctx.lineWidth = 1;
        ctx.setLineDash([3, 5]);
        ctx.beginPath();
        ctx.moveTo(mx, vy0);
        ctx.lineTo(mx, vy1);
        ctx.stroke();
        ctx.setLineDash([]);
        // Soft bead on the centre line.
        const pr = 10 + 10 * score;
        const bead = ctx.createRadialGradient(mx, my, 0, mx, my, pr);
        bead.addColorStop(0, 'rgba(220,255,255,0.9)');
        bead.addColorStop(0.4, 'rgba(140,235,255,0.45)');
        bead.addColorStop(1, 'rgba(120,220,255,0)');
        ctx.globalAlpha = a * (0.25 + 0.3 * score);
        ctx.fillStyle = bead;
        ctx.beginPath();
        ctx.arc(mx, my, pr, 0, Math.PI * 2);
        ctx.fill();
        ctx.globalAlpha = a * (0.55 + 0.3 * score);
        ctx.fillStyle = 'rgba(230,255,255,1)';
        ctx.beginPath();
        ctx.arc(mx, my, 2.4 + 1.6 * score, 0, Math.PI * 2);
        ctx.fill();
      }
    }

    // 6. Pass pulses: a bright bead running from the ball to the reception.
    // `pulse-free` and `corridor` skip the glowing dots; the lane itself
    // already encodes the probability.
    const showPulses = variant === 'classic';
    if (showPulses) for (const p of lane.pulses ?? []) {
      const u = Math.min(1, Math.max(0, p.progress ?? 0));
      const pxWorld = p.fromX + (p.toX - p.fromX) * u;
      const [px, py] = toScreen(pxWorld, y);
      const dirSign = Math.sign(p.toX - p.fromX) || 1;
      const [tx, ty] = toScreen(pxWorld - dirSign * 2.8, y);
      // Comet tail.
      ctx.globalAlpha = a * 0.3 * Math.sin(Math.PI * u);
      ctx.strokeStyle = 'rgba(180,245,255,1)';
      ctx.lineWidth = 2.4;
      ctx.beginPath();
      ctx.moveTo(tx, ty);
      ctx.lineTo(px, py);
      ctx.stroke();
      // Halo + core bead.
      ctx.globalAlpha = a * 0.16 * Math.sin(Math.PI * u);
      ctx.fillStyle = 'rgba(160,235,255,1)';
      ctx.beginPath();
      ctx.arc(px, py, 9, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = a * 0.72 * Math.sin(Math.PI * u);
      ctx.beginPath();
      ctx.arc(px, py, 3.2, 0, Math.PI * 2);
      ctx.fill();
    }
  }
  ctx.filter = 'none';
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Attack channels (thick curved run arrows) ───────────────────
export function drawAttackChannels(ctx, toScreen, channels, outerAlpha = 1, variant = 'classic') {
  if (!channels || channels.length === 0 || outerAlpha <= 0.01) return;
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  for (const ch of channels) {
    if (!ch.from || !ch.to) continue;
    const a = (ch.alpha ?? 1) * outerAlpha;
    if (a <= 0.01) continue;
    if (ch.blur > 0.05) {
      try { ctx.filter = `blur(${ch.blur.toFixed(2)}px)`; } catch { /* ignore */ }
    } else {
      ctx.filter = 'none';
    }
    const score = Number.isFinite(ch.score) ? Math.max(0, Math.min(1, ch.score)) : 0.5;
    const g = smoothstep(ch.growth ?? 1);
    const [fx, fy] = toScreen(ch.from.x, ch.from.y);
    // Grow in world space so the curve stays geometrically correct.
    const tip = {
      x: ch.from.x + (ch.to.x - ch.from.x) * g,
      y: ch.from.y + (ch.to.y - ch.from.y) * g,
    };
    const dx = ch.to.x - ch.from.x;
    const dy = ch.to.y - ch.from.y;
    const len = Math.hypot(dx, dy) || 1;
    const nx = -dy / len;
    const ny = dx / len;
    const curve = (ch.curve ?? 0) * g;
    const control = {
      x: (ch.from.x + tip.x) / 2 + nx * curve,
      y: (ch.from.y + tip.y) / 2 + ny * curve,
    };
    const [tx, ty] = toScreen(tip.x, tip.y);
    const [cx, cy] = toScreen(control.x, control.y);
    const width = 1.3 + 1.2 * score;
    // Soft glow pass.
    if (variant !== 'heat-landing') {
      ctx.globalAlpha = a * (0.09 + 0.08 * score);
      ctx.strokeStyle = 'rgba(255,190,90,1)';
      ctx.lineWidth = width * 1.9;
      ctx.beginPath();
      ctx.moveTo(fx, fy);
      ctx.quadraticCurveTo(cx, cy, tx, ty);
      ctx.stroke();
    }
    const showShaft = variant !== 'heat-landing' && variant !== 'lanes' && variant !== 'ghosts';
    if (showShaft) {
      // Main shaft.
      ctx.globalAlpha = a * (0.27 + 0.23 * score);
      ctx.lineWidth = variant === 'minimal' ? 0.8 : width;
      ctx.beginPath();
      ctx.moveTo(fx, fy);
      ctx.quadraticCurveTo(cx, cy, tx, ty);
      ctx.stroke();
    }
    if (variant === 'lanes') {
      // A run corridor: wide translucent band with a calm centerline.
      ctx.globalAlpha = a * (0.05 + 0.06 * score);
      ctx.lineWidth = 13 + 12 * score;
      ctx.beginPath();
      ctx.moveTo(fx, fy);
      ctx.quadraticCurveTo(cx, cy, tx, ty);
      ctx.stroke();
      ctx.globalAlpha = a * (0.28 + 0.24 * score);
      ctx.lineWidth = 1.2;
      ctx.beginPath();
      ctx.moveTo(fx, fy);
      ctx.quadraticCurveTo(cx, cy, tx, ty);
      ctx.stroke();
    }
    if (variant === 'ghosts') {
      // A dotted trace plus three fading echoes at the landing end.
      ctx.globalAlpha = a * 0.35;
      ctx.setLineDash([2, 5]);
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(fx, fy);
      ctx.quadraticCurveTo(cx, cy, tx, ty);
      ctx.stroke();
      ctx.setLineDash([]);
      for (let k = 1; k <= 3; k++) {
        const t = Math.max(0, 1 - k * 0.14);
        const ex = (1 - t) * (1 - t) * fx + 2 * (1 - t) * t * cx + t * t * tx;
        const ey = (1 - t) * (1 - t) * fy + 2 * (1 - t) * t * cy + t * t * ty;
        ctx.globalAlpha = a * (0.28 - k * 0.06);
        ctx.fillStyle = 'rgba(255,190,90,1)';
        ctx.beginPath();
        ctx.arc(ex, ey, 3.5 - k * 0.6, 0, Math.PI * 2);
        ctx.fill();
      }
    }
    const showArrow = variant === 'classic' || variant === 'arrows';
    // Arrowhead follows the curve tangent at the tip.
    const hx = tx - cx;
    const hy = ty - cy;
    const hl = Math.hypot(hx, hy) || 1;
    const ux = hx / hl;
    const uy = hy / hl;
    if (showArrow) {
      const head = (7 + 4 * score) * Math.max(0.18, g);
      ctx.globalAlpha = a * (0.45 + 0.25 * score);
      ctx.beginPath();
      ctx.moveTo(tx, ty);
      ctx.lineTo(tx - ux * head - uy * 4, ty - uy * head + ux * 4);
      ctx.lineTo(tx - ux * head + uy * 4, ty - uy * head - ux * 4);
      ctx.closePath();
      ctx.fillStyle = 'rgba(255,190,90,1)';
      ctx.fill();
    }
    // Anchor ring: the run belongs to this player.
    if (variant !== 'heat-landing') {
      ctx.globalAlpha = a * 0.22;
      ctx.beginPath();
      ctx.arc(fx, fy, 7 + 1.5 * g, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(255,190,90,1)';
      ctx.lineWidth = 1;
      ctx.stroke();
    }

    // Lane landing: tint the destination channel and pulse gold where the
    // run arrives. Cyan lanes answer "where can the ball go?"; gold landings
    // answer "where should this man arrive?".
    if (ch.landing && variant !== 'minimal') {
      const l = ch.landing;
      const lg = Math.max(0.15, g);
      // Anchor the glow to the arrow tip, not the raw sampled target: the
      // tracked endpoint eases, so the raw target can sit metres ahead of the
      // head during a cut. Tying them together keeps the gold arrival locked
      // to the arrow at every frame.
      const lx = tx;
      const ly = ty;
      const [tx0, tyTop] = toScreen(l.x0, l.y1);
      const [tx1, tyBottom] = toScreen(l.x1, l.y0);
      ctx.globalAlpha = a * lg * (0.012 + 0.018 * score);
      ctx.fillStyle = 'rgba(255,190,90,1)';
      ctx.fillRect(
        Math.min(tx0, tx1),
        Math.min(tyTop, tyBottom),
        Math.abs(tx1 - tx0),
        Math.abs(tyBottom - tyTop),
      );
      // Soft elliptical landing wash, elongated along the lane.
      ctx.save();
      ctx.translate(lx, ly);
      ctx.scale(1, 0.42);
      const r = 18 + 12 * score;
      const landingGrad = ctx.createRadialGradient(0, 0, 0, 0, 0, r);
      landingGrad.addColorStop(0, 'rgba(255,214,130,0.42)');
      landingGrad.addColorStop(0.55, 'rgba(255,190,90,0.16)');
      landingGrad.addColorStop(1, 'rgba(255,190,90,0)');
      ctx.globalAlpha = a * lg;
      ctx.fillStyle = landingGrad;
      ctx.beginPath();
      ctx.arc(0, 0, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
      // Bright core through the landing spot, parallel to the channel.
      const [cx0, cy0] = toScreen(tip.x - 2.6, tip.y);
      const [cx1, cy1] = toScreen(tip.x + 2.6, tip.y);
      ctx.globalAlpha = a * lg * (0.24 + 0.3 * score);
      ctx.strokeStyle = 'rgba(255,214,130,1)';
      ctx.lineWidth = 1.2 + 1.4 * score;
      ctx.beginPath();
      ctx.moveTo(cx0, cy0);
      ctx.lineTo(cx1, cy1);
      ctx.stroke();
      // Landing ring marks the exact arrival point.
      ctx.globalAlpha = a * lg * 0.35;
      ctx.beginPath();
      ctx.arc(lx, ly, 5 + 2 * score, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(255,214,130,1)';
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  }
  ctx.filter = 'none';
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Passing network (on-pitch, live player positions) ───────────
/**
 * `livePositions` maps `team:shirt` to the current pitch position. When
 * present, nodes and links are anchored there, so the network moves with the
 * players instead of sitting on historical average-touch spots. The roster
 * snapshot carries every shirt, so the network reads as the whole team:
 * pairs that have barely exchanged the ball are a whisper, pairs that keep
 * exchanging it thicken into the team's real passing channels. Links arrive
 * sorted weakest-first, so the strongest channel is always drawn on top.
 */
export function drawPassingNetwork(
  ctx,
  toScreen,
  snapshot,
  outerAlpha = 1,
  team = 0,
  livePositions = null,
  pulse = null,
) {
  if (!snapshot || !snapshot.nodes || snapshot.nodes.length === 0 || outerAlpha <= 0.01) return;
  if ((snapshot.streak ?? 0) < 3) return;
  const outlet = snapshot.mode === 'outlet';
  const maxTouches = snapshot.maxTouches || 1;
  const resolve = (node) => {
    if (!livePositions) return node;
    const key = `${node.team}:${node.shirt}`;
    const live = typeof livePositions.get === 'function'
      ? livePositions.get(key)
      : livePositions[key];
    // A departed/substituted shirt has no live body: drop it rather than
    // anchoring a link to a historical average position.
    return live ?? null;
  };
  const nodeByKey = new Map(snapshot.nodes.map((n) => [`${n.team}:${n.shirt}`, n]));
  ctx.save();
  ctx.lineCap = 'round';
  // Links first, behind the nodes. Absolute intensity: zero passes is a faint
  // thread, one pass is barely there, eight-plus is a clear channel. The scale
  // is fixed, not normalised to the strongest current link, so a pair that
  // stops combining actually thins out instead of staying the local maximum.
  const logMax = Math.log2(1 + (outlet ? 4 : 8));
  const pairSet = new Set();
  for (const l of snapshot.links ?? []) {
    const nodeA = nodeByKey.get(l.a);
    const nodeB = nodeByKey.get(l.b);
    if (!nodeA || !nodeB) continue;
    const a = resolve(nodeA);
    const b = resolve(nodeB);
    if (!a || !b) continue;
    pairSet.add(l.a < l.b ? `${l.a}|${l.b}` : `${l.b}|${l.a}`);
    const [ax, ay] = toScreen(a.x, a.y);
    const [bx, by] = toScreen(b.x, b.y);
    const t = l.n > 0 ? Math.min(1, Math.log2(1 + l.n) / logMax) : 0;
    const alpha = (outlet
      ? 0.15 + 0.5 * t
      : (l.n > 0 ? 0.05 + 0.5 * t * t : 0.032)) * outerAlpha;
    ctx.strokeStyle = team === 1 ? 'rgba(47,129,247,1)' : 'rgba(242,242,242,1)';
    // Outlet mode gets the same glow-under-core treatment as the other live
    // layers; the mesh keeps its quiet web so it can still read as a whole.
    if (outlet) {
      ctx.globalAlpha = alpha * 0.3;
      ctx.lineWidth = (1.0 + 1.9 * t) * 2.8;
      ctx.beginPath();
      ctx.moveTo(ax, ay);
      ctx.lineTo(bx, by);
      ctx.stroke();
    }
    ctx.globalAlpha = alpha;
    ctx.lineWidth = outlet ? 1.0 + 1.9 * t : (l.n > 0 ? 0.9 + 2.1 * t : 0.8);
    ctx.beginPath();
    ctx.moveTo(ax, ay);
    ctx.lineTo(bx, by);
    ctx.stroke();
  }
  // Pass pulse: one bead travelling from the passer to the receiver on the
  // wire that represents the completed pass.
  if (pulse && nodeByKey.size > 0) {
    const pair = pulse.a < pulse.b ? `${pulse.a}|${pulse.b}` : `${pulse.b}|${pulse.a}`;
    const nodeA = nodeByKey.get(pulse.a);
    const nodeB = nodeByKey.get(pulse.b);
    if (nodeA && nodeB) {
      const a = resolve(nodeA);
      const b = resolve(nodeB);
      if (a && b) {
        const [ax, ay] = toScreen(a.x, a.y);
        const [bx, by] = toScreen(b.x, b.y);
        const u = smoothstep(pulse.progress ?? 0);
        const fade = Math.sin(Math.PI * Math.min(1, Math.max(0, pulse.progress ?? 0)));
        const px = ax + (bx - ax) * u;
        const py = ay + (by - ay) * u;
        const rgb = team === 1 ? '120,185,255' : '255,255,255';
        // Outlet mode only keeps each player's top wire; a completed pass to
        // any other teammate still deserves a pulse, so give the bead a
        // transient ghost thread that fades with it.
        if (!pairSet.has(pair)) {
          ctx.globalAlpha = 0.11 * fade * outerAlpha;
          ctx.strokeStyle = `rgba(${rgb},1)`;
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(ax, ay);
          ctx.lineTo(bx, by);
          ctx.stroke();
        }
        ctx.globalAlpha = 0.2 * fade * outerAlpha;
        ctx.fillStyle = `rgba(${rgb},1)`;
        ctx.beginPath();
        ctx.arc(px, py, 8, 0, Math.PI * 2);
        ctx.fill();
        ctx.globalAlpha = 0.9 * fade * outerAlpha;
        ctx.beginPath();
        ctx.arc(px, py, 3, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }
  // Node rings sit around the live bodies (the body itself already carries
  // the shirt number, so the ring only encodes involvement volume).
  const rgb = team === 1 ? '47,129,247' : '242,242,242';
  for (const n of snapshot.nodes) {
    const liveN = resolve(n);
    if (!liveN) continue;
    const [sx, sy] = toScreen(liveN.x, liveN.y);
    const involvement = n.n > 0 ? Math.sqrt(n.n / maxTouches) : 0;
    const r = 7 + 2.5 * involvement;
    ctx.globalAlpha = (0.1 + 0.1 * involvement) * outerAlpha;
    ctx.beginPath();
    ctx.arc(sx, sy, r, 0, Math.PI * 2);
    ctx.fillStyle = `rgba(${rgb},1)`;
    ctx.fill();
    ctx.globalAlpha = (0.26 + 0.24 * involvement) * outerAlpha;
    ctx.strokeStyle = `rgba(${rgb},1)`;
    ctx.lineWidth = 1.1;
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Vision ─────────────────────────────────────────────────────
function clipPitch(ctx, toScreen) {
  const [x0, y0] = toScreen(-52.5, 34);
  const [x1, y1] = toScreen(52.5, -34);
  ctx.beginPath();
  ctx.rect(
    Math.min(x0, x1),
    Math.min(y0, y1),
    Math.abs(x1 - x0),
    Math.abs(y1 - y0),
  );
  ctx.clip();
}

function traceWorldPolygon(ctx, toScreen, points) {
  if (!points || points.length < 3) return false;
  const [sx0, sy0] = toScreen(points[0].x, points[0].y);
  ctx.beginPath();
  ctx.moveTo(sx0, sy0);
  for (let i = 1; i < points.length; i++) {
    const [sx, sy] = toScreen(points[i].x, points[i].y);
    ctx.lineTo(sx, sy);
  }
  ctx.closePath();
  return true;
}

/**
 * Long raycast vision: a faint field wash, dark shadow wedges behind every
 * blocking body, then the lit polygon filled with a radial falloff. Drawn
 * beneath the near cone so immediate awareness stays the brighter layer.
 */
export function drawVisionField(ctx, toScreen, field, outerAlpha = 1, pixelsPerMetre = 1, variant = 'classic') {
  if (!field || !field.apex || outerAlpha <= 0.01) return;
  ctx.save();
  clipPitch(ctx, toScreen);
  ctx.lineJoin = 'round';

  // 1. Potential field: the full arc at a whisper.
  if (traceWorldPolygon(ctx, toScreen, field.sector)) {
    ctx.globalAlpha = 0.05 * outerAlpha;
    ctx.fillStyle = 'rgba(80,220,180,1)';
    ctx.fill();
  }

  // 2. Occlusion: penumbra first, umbra on top. The shadow fills are clipped
  // to the lit arc, so a blocker never darkens ground the light never reached.
  // Overlapping shadows compound, so double-covered ground reads deeper.
  if (field.shadows?.length) {
    ctx.save();
    if (traceWorldPolygon(ctx, toScreen, field.sector)) ctx.clip();
    for (const shadow of field.shadows) {
      if (traceWorldPolygon(ctx, toScreen, shadow.penumbra)) {
        ctx.globalAlpha = (shadow.own ? 0.04 : 0.055) * outerAlpha;
        ctx.fillStyle = 'rgba(4,10,15,1)';
        ctx.fill();
      }
      if (traceWorldPolygon(ctx, toScreen, shadow.points)) {
        ctx.globalAlpha = (shadow.own ? 0.14 : 0.18) * outerAlpha;
        ctx.fillStyle = 'rgba(4,10,15,1)';
        ctx.fill();
      }
    }
    ctx.restore();
  }

  // 3. Lit polygon: distance falloff from the radial gradient multiplied by
  // the per-ray angular falloff. Wedges are grouped into brightness buckets,
  // so 64 rays cost a handful of fills instead of 64.
  const [ax, ay] = toScreen(field.apex.x, field.apex.y);
  const pts = field.polygon;
  if (pts && pts.length >= 3) {
    const radiusPx = Math.max(1, field.reach * pixelsPerMetre);
    const grad = ctx.createRadialGradient(ax, ay, 2, ax, ay, radiusPx);
    grad.addColorStop(0, 'rgba(120,255,220,0.9)');
    grad.addColorStop(0.55, 'rgba(80,220,180,0.5)');
    grad.addColorStop(1, 'rgba(80,220,180,0.08)');
    ctx.fillStyle = grad;
    const falloff = field.falloff ?? [];
    const buckets = new Map();
    for (let i = 1; i < pts.length - 1; i++) {
      const f = ((falloff[i] ?? 1) + (falloff[i + 1] ?? 1)) / 2;
      const level = Math.max(1, Math.round(f * 32)) / 32;
      const list = buckets.get(level) ?? [];
      list.push([pts[i], pts[i + 1]]);
      buckets.set(level, list);
    }
    for (const [level, pairs] of [...buckets.entries()].sort((a, b) => a[0] - b[0])) {
      ctx.globalAlpha = outerAlpha * 0.2 * level;
      ctx.beginPath();
      for (const [p1, p2] of pairs) {
        const [x1, y1] = toScreen(p1.x, p1.y);
        const [x2, y2] = toScreen(p2.x, p2.y);
        ctx.moveTo(ax, ay);
        ctx.lineTo(x1, y1);
        ctx.lineTo(x2, y2);
        ctx.closePath();
      }
      ctx.fill();
    }
    // 4. Crisp light boundary: the analytical edge of the visible field.
    if (traceWorldPolygon(ctx, toScreen, field.polygon)) {
      ctx.globalAlpha = 0.22 * outerAlpha;
      ctx.strokeStyle = 'rgba(120,255,220,1)';
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  }
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Vision cone ─────────────────────────────────────────────────
export function drawVisionCone(ctx, toScreen, cone, outerAlpha = 1, variant = 'classic') {
  if (!cone || !cone.apex || outerAlpha <= 0.01) return;
  const scale = variant === 'soft-wedge' ? 0.5 : variant === 'minimal' ? 0.6 : variant === 'spotlight' ? 0.75 : 1;
  outerAlpha = outerAlpha * scale;
  // Visible cone: the same angular falloff as the long field, at the near
  // radius. Grouped wedges keep the soft edge without 12 separate fills.
  ctx.save();
  // Keep the wash inside the pitch rectangle.
  clipPitch(ctx, toScreen);

  if (variant === 'minimal') {
    // Outline only: the shape of sight without the wash. Useful on top of a
    // busy heatmap or when the analyst only needs the boundary.
    if (traceWorldPolygon(ctx, toScreen, cone.conePoints ?? [])) {
      ctx.globalAlpha = 0.5 * outerAlpha;
      ctx.strokeStyle = 'rgba(120,255,220,0.9)';
      ctx.lineWidth = 1.2;
      ctx.stroke();
    }
    const [axm, aym] = toScreen(cone.apex.x, cone.apex.y);
    const [fxm, fym] = toScreen(
      cone.apex.x + Math.cos(cone.facing ?? 0) * (cone.radius ?? 12),
      cone.apex.y + Math.sin(cone.facing ?? 0) * (cone.radius ?? 12),
    );
    ctx.globalAlpha = 0.45 * outerAlpha;
    ctx.beginPath();
    ctx.moveTo(axm, aym);
    ctx.lineTo(fxm, fym);
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.restore();
    return;
  }

  if (variant === 'spotlight') {
    // A soft pool of attention around the carrier, with a single orientation
    // tick. Reads immediately without drawing a hard wedge over the pitch.
    const [axs, ays] = toScreen(cone.apex.x, cone.apex.y);
    const [pxs, pys] = toScreen(
      cone.apex.x + Math.cos(cone.facing ?? 0) * (cone.radius ?? 12),
      cone.apex.y + Math.sin(cone.facing ?? 0) * (cone.radius ?? 12),
    );
    const r = Math.max(8, Math.hypot(pxs - axs, pys - ays));
    const grad = ctx.createRadialGradient(axs, ays, 2, axs, ays, r);
    grad.addColorStop(0, 'rgba(120,255,220,0.28)');
    grad.addColorStop(0.55, 'rgba(80,220,180,0.10)');
    grad.addColorStop(1, 'rgba(80,220,180,0)');
    ctx.globalAlpha = outerAlpha;
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.arc(axs, ays, r, 0, Math.PI * 2);
    ctx.fill();
    ctx.globalAlpha = 0.35 * outerAlpha;
    ctx.strokeStyle = 'rgba(120,255,220,0.9)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(axs, ays);
    ctx.lineTo(pxs, pys);
    ctx.stroke();
    ctx.globalAlpha = 1;
    ctx.restore();
    return;
  }

  if (variant === 'soft-wedge') {
    try { ctx.filter = 'blur(2.5px)'; } catch { /* ignore */ }
  }
  const pts = cone.conePoints ?? [];
  const coneFalloff = cone.coneFalloff ?? [];
  if (pts.length >= 3) {
    if (coneFalloff.length >= 2) {
      const [ax, ay] = toScreen(pts[0].x, pts[0].y);
      const buckets = new Map();
      for (let i = 1; i < pts.length - 2; i++) {
        const f = ((coneFalloff[i - 1] ?? 1) + (coneFalloff[i] ?? 1)) / 2;
        const level = Math.max(1, Math.round(f * 32)) / 32;
        const list = buckets.get(level) ?? [];
        list.push([pts[i], pts[i + 1]]);
        buckets.set(level, list);
      }
      ctx.fillStyle = 'rgba(80,220,180,1)';
      for (const [level, pairs] of [...buckets.entries()].sort((a, b) => a[0] - b[0])) {
        ctx.globalAlpha = 0.14 * outerAlpha * level;
        ctx.beginPath();
        for (const [p1, p2] of pairs) {
          const [x1, y1] = toScreen(p1.x, p1.y);
          const [x2, y2] = toScreen(p2.x, p2.y);
          ctx.moveTo(ax, ay);
          ctx.lineTo(x1, y1);
          ctx.lineTo(x2, y2);
          ctx.closePath();
        }
        ctx.fill();
      }
    } else {
      ctx.globalAlpha = 0.16 * outerAlpha;
      ctx.fillStyle = 'rgba(80,220,180,1)';
      ctx.beginPath();
      const [sx0, sy0] = toScreen(pts[0].x, pts[0].y);
      ctx.moveTo(sx0, sy0);
      for (let i = 1; i < pts.length; i++) {
        const [sx, sy] = toScreen(pts[i].x, pts[i].y);
        ctx.lineTo(sx, sy);
      }
      ctx.closePath();
      ctx.fill();
    }
  }
  // blind sector: drawn in world coords then projected. Kept light so the
  // long raycast field (and its shadows) still reads through the near circle.
  const bpts = cone.blindConePoints ?? [];
  const showBlind = variant === 'classic' || variant === 'spotlight';
  const showTeammateLines = variant === 'classic';
  if (showBlind && bpts.length > 0) {
    ctx.globalAlpha = 0.03 * outerAlpha;
    ctx.fillStyle = 'rgba(255,110,110,1)';
    ctx.beginPath();
    const [bsx0, bsy0] = toScreen(bpts[0].x, bpts[0].y);
    ctx.moveTo(bsx0, bsy0);
    for (let i = 1; i < bpts.length; i++) {
      const [bsx, bsy] = toScreen(bpts[i].x, bpts[i].y);
      ctx.lineTo(bsx, bsy);
    }
    ctx.closePath();
    ctx.fill();
  }
  ctx.filter = 'none';
  ctx.restore();
  // redraw visible cone solid on top of blind wash is handled by first fill; instead draw lines.
  if (showTeammateLines) {
  ctx.save();
  const [ax2, ay2] = toScreen(cone.apex.x, cone.apex.y);
  // visible teammates: solid line in teal
  ctx.globalAlpha = 0.4 * outerAlpha;
  ctx.strokeStyle = 'rgba(80,220,180,1)';
  ctx.lineWidth = 1.2;
  for (const t of cone.visible ?? []) {
    const [tx, ty] = toScreen(t.x, t.y);
    ctx.beginPath();
    ctx.moveTo(ax2, ay2);
    ctx.lineTo(tx, ty);
    ctx.stroke();
  }
  // blind teammates: dotted line in red
  ctx.globalAlpha = 0.4 * outerAlpha;
  ctx.strokeStyle = 'rgba(255,110,110,1)';
  ctx.setLineDash([3, 3]);
  ctx.lineWidth = 1;
  for (const t of cone.blind ?? []) {
    const [tx, ty] = toScreen(t.x, t.y);
    ctx.beginPath();
    ctx.moveTo(ax2, ay2);
    ctx.lineTo(tx, ty);
    ctx.stroke();
  }
  ctx.setLineDash([]);
  ctx.globalAlpha = 1;
  if (showTeammateLines) ctx.restore();
  }

  ctx.globalAlpha = 1;
}

// ── One-shot event effects ─────────────────────────────────────
export function drawEventFx(ctx, toScreen, effects) {
  if (!effects || effects.length === 0) return;
  ctx.save();
  for (const e of effects) {
    const [sx, sy] = toScreen(e.x, e.y);
    const p = e.progress ?? 0;
    const alpha = 1 - p;
    if (e.kind === 'save') {
      ctx.globalAlpha = alpha * 0.8;
      ctx.beginPath();
      ctx.arc(sx, sy, 6 + p * 12, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(120,220,160,0.9)';
      ctx.lineWidth = 1.5;
      ctx.stroke();
    } else if (e.kind === 'post') {
      ctx.globalAlpha = alpha * 0.8;
      ctx.beginPath();
      ctx.arc(sx, sy, 4 + p * 8, 0, Math.PI * 2);
      ctx.strokeStyle = 'rgba(255,200,120,0.9)';
      ctx.lineWidth = 1.5;
      ctx.stroke();
    } else if (e.kind === 'flag') {
      ctx.globalAlpha = alpha * 0.9;
      ctx.fillStyle = 'rgba(255,100,100,0.9)';
      ctx.font = '700 10px ui-sans-serif, sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('FLAG', sx, sy - 12);
    }
  }
  ctx.globalAlpha = 1;
  ctx.restore();
}
