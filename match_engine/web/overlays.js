// Canvas drawing for the team overlays: heatmap, offside line, broadcast analysis.

import { smoothstep } from './easing.js';

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
  { canvas: null, ctx: null, version: -1, cols: 0, rows: 0 },
  { canvas: null, ctx: null, version: -1, cols: 0, rows: 0 },
];

export function drawHeatmap(ctx, rect, heat, team, alpha = 1) {
  if (!heat || heat.max <= 0 || alpha <= 0.01) return;
  const slot = team === 1 ? 1 : 0;
  const cache = heatCaches[slot];
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

export function drawAnalysis(ctx, toScreen, players, team, alpha, ball, possessionTeam, options = {}) {
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
    if (presser2 && pressD2 < 15) {
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

// ── Passing lanes (live arrows anchored to ball + receiver) ─────
export function drawPassingLanes(ctx, toScreen, lanes, outerAlpha = 1) {
  if (!lanes || lanes.length === 0 || outerAlpha <= 0.01) return;
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  for (const lane of lanes) {
    if (!lane.from || !lane.to) continue;
    const a = (lane.alpha ?? 1) * outerAlpha;
    if (a <= 0.01) continue;
    if (lane.blur > 0.05) {
      try { ctx.filter = `blur(${lane.blur.toFixed(2)}px)`; } catch { /* ignore */ }
    } else {
      ctx.filter = 'none';
    }
    const [fx, fy] = toScreen(lane.from.x, lane.from.y);
    const [ex, ey] = toScreen(lane.to.x, lane.to.y);
    // Grow from the ball to the live receiver position.
    const g = smoothstep(lane.growth ?? 1);
    const tx = fx + (ex - fx) * g;
    const ty = fy + (ey - fy) * g;
    const dx = ex - fx;
    const dy = ey - fy;
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx / len;
    const uy = dy / len;
    const p = Number.isFinite(lane.p) ? Math.max(0, Math.min(1, lane.p)) : 0.5;
    const width = 1.1 + 1.3 * p;
    // Soft under-stroke widens the arrow without turning it opaque.
    ctx.globalAlpha = a * (0.08 + 0.1 * p);
    ctx.strokeStyle = 'rgba(120,220,255,1)';
    ctx.lineWidth = width * 2.2;
    ctx.beginPath();
    ctx.moveTo(fx, fy);
    ctx.lineTo(tx, ty);
    ctx.stroke();
    // Main shaft.
    ctx.globalAlpha = a * (0.25 + 0.27 * p);
    ctx.lineWidth = width;
    ctx.beginPath();
    ctx.moveTo(fx, fy);
    ctx.lineTo(tx, ty);
    ctx.stroke();
    // Arrowhead sitting on the receiver; small at the start of the growth.
    const head = (6 + 4 * p) * Math.max(0.18, g);
    ctx.globalAlpha = a * (0.36 + 0.22 * p);
    ctx.beginPath();
    ctx.moveTo(tx, ty);
    ctx.lineTo(tx - ux * head - uy * 4, ty - uy * head + ux * 4);
    ctx.lineTo(tx - ux * head + uy * 4, ty - uy * head - ux * 4);
    ctx.closePath();
    ctx.fillStyle = 'rgba(120,220,255,1)';
    ctx.fill();
    // Origin ring shows the lane belongs to the ball/carrier.
    ctx.globalAlpha = a * 0.18;
    ctx.beginPath();
    ctx.arc(fx, fy, 6 + 1.5 * g, 0, Math.PI * 2);
    ctx.strokeStyle = 'rgba(120,220,255,1)';
    ctx.lineWidth = 1;
    ctx.stroke();
  }
  ctx.filter = 'none';
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Attack channels (thick curved run arrows) ───────────────────
export function drawAttackChannels(ctx, toScreen, channels, outerAlpha = 1) {
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
    ctx.globalAlpha = a * (0.09 + 0.08 * score);
    ctx.strokeStyle = 'rgba(255,190,90,1)';
    ctx.lineWidth = width * 1.9;
    ctx.beginPath();
    ctx.moveTo(fx, fy);
    ctx.quadraticCurveTo(cx, cy, tx, ty);
    ctx.stroke();
    // Main shaft.
    ctx.globalAlpha = a * (0.27 + 0.23 * score);
    ctx.lineWidth = width;
    ctx.beginPath();
    ctx.moveTo(fx, fy);
    ctx.quadraticCurveTo(cx, cy, tx, ty);
    ctx.stroke();
    // Arrowhead follows the curve tangent at the tip.
    const hx = tx - cx;
    const hy = ty - cy;
    const hl = Math.hypot(hx, hy) || 1;
    const ux = hx / hl;
    const uy = hy / hl;
    const head = (7 + 4 * score) * Math.max(0.18, g);
    ctx.globalAlpha = a * (0.45 + 0.25 * score);
    ctx.beginPath();
    ctx.moveTo(tx, ty);
    ctx.lineTo(tx - ux * head - uy * 4, ty - uy * head + ux * 4);
    ctx.lineTo(tx - ux * head + uy * 4, ty - uy * head - ux * 4);
    ctx.closePath();
    ctx.fillStyle = 'rgba(255,190,90,1)';
    ctx.fill();
    // Anchor ring: the run belongs to this player.
    ctx.globalAlpha = a * 0.22;
    ctx.beginPath();
    ctx.arc(fx, fy, 7 + 1.5 * g, 0, Math.PI * 2);
    ctx.strokeStyle = 'rgba(255,190,90,1)';
    ctx.lineWidth = 1;
    ctx.stroke();
  }
  ctx.filter = 'none';
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Passing network (on-pitch, live player positions) ───────────
/**
 * `livePositions` maps `team:shirt` to the current pitch position. When
 * present, nodes and links are anchored there, so the network moves with the
 * players instead of sitting on historical average-touch spots. Link
 * intensity is absolute (based on the pair's pass count), not relative to the
 * strongest link: a new link is barely visible and darkens only as those two
 * players keep exchanging the ball.
 */
export function drawPassingNetwork(
  ctx,
  toScreen,
  snapshot,
  outerAlpha = 1,
  team = 0,
  livePositions = null,
) {
  if (!snapshot || !snapshot.nodes || snapshot.nodes.length === 0 || outerAlpha <= 0.01) return;
  if ((snapshot.streak ?? 0) < 3) return;
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
  // Links first, behind the nodes.
  for (const l of snapshot.links ?? []) {
    const nodeA = nodeByKey.get(l.a);
    const nodeB = nodeByKey.get(l.b);
    if (!nodeA || !nodeB) continue;
    const a = resolve(nodeA);
    const b = resolve(nodeB);
    if (!a || !b) continue;
    const [ax, ay] = toScreen(a.x, a.y);
    const [bx, by] = toScreen(b.x, b.y);
    // Absolute intensity: count 1 is a whisper, count 7+ is a clear line.
    const intensity = Math.min(1, Math.log2(1 + l.n) / 3);
    const alpha = (0.07 + 0.34 * intensity) * outerAlpha;
    ctx.globalAlpha = alpha;
    ctx.strokeStyle = team === 1 ? 'rgba(47,129,247,1)' : 'rgba(242,242,242,1)';
    ctx.lineWidth = 1 + 1.6 * intensity;
    ctx.beginPath();
    ctx.moveTo(ax, ay);
    ctx.lineTo(bx, by);
    ctx.stroke();
  }
  // Node rings sit around the live bodies (the body itself already carries
  // the shirt number, so the ring only encodes involvement volume).
  const rgb = team === 1 ? '47,129,247' : '242,242,242';
  for (const n of snapshot.nodes) {
    const liveN = resolve(n);
    if (!liveN) continue;
    const [sx, sy] = toScreen(liveN.x, liveN.y);
    const r = 7 + 2.5 * Math.sqrt(n.n / maxTouches);
    ctx.globalAlpha = 0.14 * outerAlpha;
    ctx.beginPath();
    ctx.arc(sx, sy, r, 0, Math.PI * 2);
    ctx.fillStyle = `rgba(${rgb},1)`;
    ctx.fill();
    ctx.globalAlpha = 0.42 * outerAlpha;
    ctx.strokeStyle = `rgba(${rgb},1)`;
    ctx.lineWidth = 1.1;
    ctx.stroke();
  }
  ctx.globalAlpha = 1;
  ctx.restore();
}

// ── Vision cone ─────────────────────────────────────────────────
export function drawVisionCone(ctx, toScreen, cone, outerAlpha = 1) {
  if (!cone || !cone.apex || outerAlpha <= 0.01) return;
  // visible cone sector
  ctx.save();
  // Keep the wash inside the pitch rectangle.
  const [pcx0, pcy0] = toScreen(-52.5, 34);
  const [pcx1, pcy1] = toScreen(52.5, -34);
  ctx.beginPath();
  ctx.rect(
    Math.min(pcx0, pcx1),
    Math.min(pcy0, pcy1),
    Math.abs(pcx1 - pcx0),
    Math.abs(pcy1 - pcy0),
  );
  ctx.clip();
  ctx.globalAlpha = 0.16 * outerAlpha;
  ctx.fillStyle = 'rgba(80,220,180,1)';
  ctx.beginPath();
  const pts = cone.conePoints ?? [];
  if (pts.length > 0) {
    const [sx0, sy0] = toScreen(pts[0].x, pts[0].y);
    ctx.moveTo(sx0, sy0);
    for (let i = 1; i < pts.length; i++) {
      const [sx, sy] = toScreen(pts[i].x, pts[i].y);
      ctx.lineTo(sx, sy);
    }
    ctx.closePath();
    ctx.fill();
  }
  // blind sector: drawn in world coords then projected
  const bpts = cone.blindConePoints ?? [];
  if (bpts.length > 0) {
    ctx.globalAlpha = 0.08 * outerAlpha;
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
  ctx.restore();
  // redraw visible cone solid on top of blind wash is handled by first fill; instead draw lines.
  ctx.save();
  const [ax2, ay2] = toScreen(cone.apex.x, cone.apex.y);
  // visible teammates: solid line in teal
  ctx.globalAlpha = 0.55 * outerAlpha;
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
  ctx.restore();
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
