import { clamp01, smoothstep } from './easing.js';

export const PASSING_LANE = {
  growIn: 0.28,
  targetTau: 0.22,
  fadeOut: 0.45,
  maxBlur: 3.5,
};

/**
 * Live passing-lane tracker.
 *
 * `updateLive(dt, computed, resolve)` receives the latest `passFan` result and
 * keeps a lane per receiver. Endpoints are re-resolved every frame, so the
 * arrows stay attached to the moving ball and receiver instead of freezing at
 * the spawn position. Lanes ease in, grow from the ball to the receiver, and
 * blur/fade out when the option disappears.
 */
export function createPassingLaneTracker(options = {}) {
  const growIn = options.growIn ?? PASSING_LANE.growIn;
  const targetTau = options.targetTau ?? PASSING_LANE.targetTau;
  const fadeOut = options.fadeOut ?? PASSING_LANE.fadeOut;
  const maxBlur = options.maxBlur ?? PASSING_LANE.maxBlur;
  const entries = new Map();

  function laneKey(lane) {
    if (lane.player) {
      const team = lane.player.team ?? 0;
      const shirt = lane.player.shirt ?? 0;
      return `${team}:${shirt}`;
    }
    return `${lane.to.x.toFixed(1)}:${lane.to.y.toFixed(1)}`;
  }

  function updateLive(dt, computed, resolve) {
    const step = Number.isFinite(dt) && dt > 0 ? dt : 0;
    const seen = new Set();
    for (const lane of computed ?? []) {
      if (!lane || !lane.to) continue;
      const key = laneKey(lane);
      seen.add(key);
      const live = resolve ? resolve(lane) : null;
      const from = live?.from ?? lane.from;
      const to = live?.to ?? lane.to;
      let e = entries.get(key);
      if (!e) {
        e = {
          key,
          player: lane.player,
          from: { x: from.x, y: from.y },
          to: { x: from.x, y: from.y },
          target: { x: to.x, y: to.y },
          growth: 0,
          alpha: 0,
          blur: 0,
          p: lane.p,
        };
        entries.set(key, e);
      }
      e.player = lane.player;
      e.from.x = from.x;
      e.from.y = from.y;
      e.target.x = to.x;
      e.target.y = to.y;
      const k = 1 - Math.exp(-step / targetTau);
      e.to.x += (e.target.x - e.to.x) * k;
      e.to.y += (e.target.y - e.to.y) * k;
      e.p = lane.p;
      e.growth = Math.min(1, e.growth + step / growIn);
      e.alpha = Math.min(1, e.alpha + step / (growIn * 0.8));
      e.blur = Math.max(0, e.blur - (step / fadeOut) * maxBlur);
    }
    for (const [key, e] of entries) {
      if (seen.has(key)) continue;
      e.growth = Math.max(0, e.growth - step / fadeOut);
      e.alpha = Math.max(0, e.alpha - step / fadeOut);
      e.blur = Math.min(maxBlur, e.blur + (step / fadeOut) * maxBlur);
      if (e.alpha <= 0.01) entries.delete(key);
    }
    return [...entries.values()].map((e) => ({
      key: e.key,
      player: e.player,
      from: { ...e.from },
      to: { ...e.to },
      growth: smoothstep(e.growth),
      alpha: clamp01(e.alpha),
      blur: e.blur,
      p: e.p,
    }));
  }

  function reset() {
    entries.clear();
  }

  return { updateLive, reset, get activeCount() { return entries.size; } };
}