// Dribble-zone layer for the main pitch: zone classification, activity heat
// and per-attempt markers (✓ success with direction, ✗ failure, ○ carry).
//
// Pure drawing over a `toScreen` transform, so the headless Canvas harness
// in dribbleOverlay.test.mjs covers it exactly like the other overlays.

import { ZONE_COLS, ZONE_ROWS, zoneWorldRect } from './dribbles.js';

const GREEN = 'rgba(80, 220, 140, 0.92)';
const RED = 'rgba(255, 92, 92, 0.95)';
const BLUE = 'rgba(110, 180, 255, 0.95)';

/**
 * Zone fill colour: hue carries *activity* (green quiet → yellow moderate →
 * red hot, per the brief), opacity carries *outcome* (a zone that keeps
 * failing reads faint, a zone that succeeds reads solid).
 */
export function zoneFillStyle(attempts, maxAttempts, success, baseAlpha = 0.55) {
  const freq = Math.min(1, (attempts / Math.max(1, maxAttempts)));
  const hue = 120 - 120 * freq;
  const a = success == null ? baseAlpha * 0.55 : 0.18 + (baseAlpha - 0.18) * success;
  return `hsla(${hue.toFixed(0)}, 82%, 52%, ${a.toFixed(3)})`;
}

function markerArrow(ctx, sx, sy, tx, ty) {
  const dx = tx - sx;
  const dy = ty - sy;
  const len = Math.hypot(dx, dy);
  if (len < 3) return false;
  ctx.beginPath();
  ctx.moveTo(sx, sy);
  ctx.lineTo(tx, ty);
  ctx.stroke();
  // Arrowhead.
  const ux = dx / len;
  const uy = dy / len;
  const head = Math.min(7, len * 0.45);
  const wing = 0.55;
  ctx.beginPath();
  ctx.moveTo(tx, ty);
  ctx.lineTo(tx - head * (ux - uy * wing), ty - head * (uy + ux * wing));
  ctx.moveTo(tx, ty);
  ctx.lineTo(tx - head * (ux + uy * wing), ty - head * (uy - ux * wing));
  ctx.stroke();
  return true;
}

function markerCheck(ctx, sx, sy) {
  ctx.beginPath();
  ctx.moveTo(sx - 3.5, sy);
  ctx.lineTo(sx - 1, sy + 2.8);
  ctx.lineTo(sx + 4, sy - 3.2);
  ctx.stroke();
}

function markerCross(ctx, sx, sy) {
  ctx.beginPath();
  ctx.moveTo(sx - 3.4, sy - 3.4);
  ctx.lineTo(sx + 3.4, sy + 3.4);
  ctx.moveTo(sx + 3.4, sy - 3.4);
  ctx.lineTo(sx - 3.4, sy + 3.4);
  ctx.stroke();
}

/**
 * Draw the layer for one selection (a team or a single player).
 *
 * @param ctx Canvas 2D context of the pitch.
 * @param toScreen world → screen transform.
 * @param stats result of `statsFor(...)` (carries `dribbles`).
 * @param team team whose attacking frame the zones are defined in.
 * @param opts `{ alpha, markers, numbers, hoverZone, selectedZone, baseAlpha }`
 */
export function drawDribbleLayer(ctx, toScreen, stats, team, opts = {}) {
  if (!stats) return;
  const alpha = opts.alpha ?? 1;
  if (alpha <= 0.01) return;
  const markers = opts.markers !== false;
  const numbers = opts.numbers !== false;
  const baseAlpha = opts.baseAlpha ?? 0.55;
  const maxAttempts = stats.zones.reduce((m, z) => Math.max(m, z.attempts), 0);

  ctx.save();
  ctx.globalAlpha = alpha;

  // 1. Activity heat per zone (faint empty zones stay unwashed).
  for (let z = 0; z < stats.zones.length; z++) {
    const zone = stats.zones[z];
    if (!zone.attempts) continue;
    const rect = zoneWorldRect(z + 1, team);
    const [sx, syTop] = toScreen(rect.x0, rect.y1);
    const [ex, syBottom] = toScreen(rect.x1, rect.y0);
    ctx.fillStyle = zoneFillStyle(zone.attempts, maxAttempts, zone.success, baseAlpha);
    ctx.fillRect(sx, syTop, ex - sx, syBottom - syTop);
  }

  // 2. Zone classification grid over the whole pitch.
  ctx.strokeStyle = 'rgba(233, 242, 255, 0.16)';
  ctx.lineWidth = 1;
  for (let row = 0; row < ZONE_ROWS; row++) {
    for (let col = 0; col < ZONE_COLS; col++) {
      const zoneId = row * ZONE_COLS + col + 1;
      const rect = zoneWorldRect(zoneId, team);
      const [sx, syTop] = toScreen(rect.x0, rect.y1);
      const [ex, syBottom] = toScreen(rect.x1, rect.y0);
      ctx.strokeRect(sx, syTop, ex - sx, syBottom - syTop);
    }
  }

  // 3. Zone ids, so the panel and the pitch can refer to the same cell.
  // A dark halo keeps them legible over the heat washes.
  ctx.fillStyle = 'rgba(233, 242, 255, 0.72)';
  ctx.font = '9px ui-sans-serif, system-ui, sans-serif';
  ctx.textAlign = 'left';
  ctx.textBaseline = 'top';
  ctx.save();
  ctx.shadowColor = 'rgba(8, 12, 18, 0.85)';
  ctx.shadowBlur = 3;
  for (let z = 1; z <= stats.zones.length; z++) {
    const rect = zoneWorldRect(z, team);
    const [sx, sy] = toScreen(rect.x0 + 1.2, rect.y1 - 1.4);
    ctx.fillText(String(z), sx, sy);
  }
  ctx.restore();

  // 4. Hover / selection outlines.
  const outline = (zoneId, color, width) => {
    const rect = zoneWorldRect(zoneId, team);
    const [sx, syTop] = toScreen(rect.x0, rect.y1);
    const [ex, syBottom] = toScreen(rect.x1, rect.y0);
    ctx.strokeStyle = color;
    ctx.lineWidth = width;
    ctx.strokeRect(sx, syTop, ex - sx, syBottom - syTop);
  };
  if (opts.hoverZone) outline(opts.hoverZone, 'rgba(47, 129, 247, 0.75)', 1.5);
  if (opts.selectedZone) outline(opts.selectedZone, 'rgba(255, 190, 90, 0.95)', 2);

  // 5. Attempt markers. Numbers are the marker's position in *this* list —
  // derived here, never stored on the shared dribble objects, so other
  // views' stats can't renumber the pitch mid-session.
  if (markers) {
    const list = stats.dribbles ?? [];
    const numbered = numbers && list.length <= 20;
    ctx.lineCap = 'round';
    for (let i = 0; i < list.length; i++) {
      const d = list[i];
      const [sx, sy] = toScreen(d.x, d.y);
      if (!d.ok) {
        ctx.strokeStyle = RED;
        ctx.lineWidth = 2;
        markerCross(ctx, sx, sy);
      } else if (d.kind === 'takeon') {
        ctx.strokeStyle = GREEN;
        ctx.lineWidth = 2;
        const hasArrow = d.arrow && (Math.abs(d.arrow.x - d.x) > 0.4 || Math.abs(d.arrow.y - d.y) > 0.4);
        if (hasArrow) {
          const [tx, ty] = toScreen(d.arrow.x, d.arrow.y);
          if (!markerArrow(ctx, sx, sy, tx, ty)) markerCheck(ctx, sx, sy);
        } else {
          markerCheck(ctx, sx, sy);
        }
      } else {
        ctx.strokeStyle = BLUE;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(sx, sy, 4, 0, Math.PI * 2);
        ctx.stroke();
      }
      if (numbered) {
        ctx.fillStyle = 'rgba(233, 242, 255, 0.8)';
        ctx.font = '8px ui-sans-serif, system-ui, sans-serif';
        ctx.textAlign = 'left';
        ctx.textBaseline = 'bottom';
        ctx.fillText(String(i + 1), sx + 5, sy - 4);
      }
    }
  }

  ctx.restore();
}

/**
 * Small legend rows for the layer (`{color, label}`), shared by the canvas
 * legend in index.html and the panel.
 */
export const DRIBBLE_LEGEND = Object.freeze([
  Object.freeze({ color: '#50dc8c', label: 'Successful take-on' }),
  Object.freeze({ color: '#ff5c5c', label: 'Lost dribble' }),
  Object.freeze({ color: '#6eb4ff', label: 'Progressive carry' }),
]);
