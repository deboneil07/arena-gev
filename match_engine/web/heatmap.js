// Cumulative per-team heatmap: pure density accumulation over the pitch.
//
// No DOM and no Canvas here so the model can be unit-tested and sampled from
// Node. The renderer owns the offscreen image and colour ramp.

const PITCH_W = 105.0;
const PITCH_H = 68.0;

/**
 * One team's density field. Samples splat a small Gaussian kernel around the
 * player's pitch cell (row 0 = world y +34, i.e. screen top).
 */
export function createHeatmap({
  cols = 42,
  rows = 28,
  sigmaBins = 1.2,
  radiusBins = 3,
} = {}) {
  const grid = new Float32Array(cols * rows);
  let version = 0;
  let max = 0;

  const kernel = [];
  const denom = 2 * sigmaBins * sigmaBins;
  for (let dy = -radiusBins; dy <= radiusBins; dy++) {
    for (let dx = -radiusBins; dx <= radiusBins; dx++) {
      const w = Math.exp(-(dx * dx + dy * dy) / denom);
      if (w > 0.01) kernel.push({ dx, dy, w });
    }
  }

  function sample(points, weight = 1) {
    for (const p of points ?? []) {
      if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
      const cx = Math.min(
        cols - 1,
        Math.max(0, Math.floor(((p.x + PITCH_W / 2) / PITCH_W) * cols)),
      );
      const cy = Math.min(
        rows - 1,
        Math.max(0, Math.floor(((PITCH_H / 2 - p.y) / PITCH_H) * rows)),
      );
      for (const k of kernel) {
        const x = cx + k.dx;
        const y = cy + k.dy;
        if (x < 0 || x >= cols || y < 0 || y >= rows) continue;
        const i = y * cols + x;
        grid[i] += k.w * weight;
        if (grid[i] > max) max = grid[i];
      }
    }
    version++;
  }

  function reset() {
    grid.fill(0);
    max = 0;
    version++;
  }

  return {
    cols,
    rows,
    grid,
    sample,
    reset,
    get max() {
      return max;
    },
    get version() {
      return version;
    },
  };
}
