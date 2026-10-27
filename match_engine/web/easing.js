// Shared easing helpers for the renderer-only animation trackers.
export function clamp01(v) {
  if (!Number.isFinite(v)) return 0;
  return Math.min(1, Math.max(0, v));
}

export function smoothstep(v) {
  const t = clamp01(v);
  return t * t * (3 - 2 * t);
}

export function easeOutQuart(v) {
  const t = clamp01(v);
  return 1 - Math.pow(1 - t, 4);
}

export function easeInOutCubic(v) {
  const t = clamp01(v);
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}