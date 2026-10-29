import { clamp01, easeOutQuart, smoothstep } from './easing.js';

export const ATTACK_LANE = {
  growIn: 0.42,
  targetTau: 0.22,
  fadeOut: 0.55,
  hold: 2.4,
  maxBlur: 3.0,
  pulseDuration: 0.62,
};

/**
 * Live tracker for the four attacking channels.
 *
 * Lanes are keyed by band index, so their geometry is stable while their
 * brightness eases toward the latest momentum/option score. They grow in from
 * the halfway line and blur-fade when the carrier leaves the attacking half.
 * A completed pass can add a pulse that travels down the destination lane.
 */
export function createAttackLaneTracker(options = {}) {
  const growIn = options.growIn ?? ATTACK_LANE.growIn;
  const targetTau = options.targetTau ?? ATTACK_LANE.targetTau;
  const fadeOut = options.fadeOut ?? ATTACK_LANE.fadeOut;
  let holdSeconds = options.hold ?? ATTACK_LANE.hold;
  const maxBlur = options.maxBlur ?? ATTACK_LANE.maxBlur;
  const pulseDuration = options.pulseDuration ?? ATTACK_LANE.pulseDuration;
  const entries = new Map();
  let pulses = [];

  function pulse(lane, fromX, toX, duration = pulseDuration) {
    if (!Number.isFinite(lane) || !Number.isFinite(fromX) || !Number.isFinite(toX)) return;
    pulses.push({ lane, fromX, toX, age: 0, duration: Math.max(0.1, duration) });
    // A burst of passes should never accumulate an unbounded trail.
    if (pulses.length > 8) pulses = pulses.slice(-8);
  }

  function updateLive(dt, state) {
    const step = Number.isFinite(dt) && dt > 0 ? dt : 0;
    const team = Number.isFinite(state?.team) ? state.team : 0;
    const lanes = state?.active ? state.lanes ?? [] : [];
    const projectedPoint =
      state?.projected &&
      Number.isFinite(state.projected.x) &&
      Number.isFinite(state.projected.y)
        ? { x: state.projected.x, y: state.projected.y }
        : null;
    const seen = new Set();

    for (const lane of lanes) {
      const key = `${team}:${lane.index}`;
      seen.add(key);
      let e = entries.get(key);
      if (!e) {
        e = {
          key,
          team,
          index: lane.index,
          y0: lane.y0,
          y1: lane.y1,
          centerY: lane.centerY,
          x0: lane.x0,
          x1: lane.x1,
          score: 0,
          targetScore: 0,
          momentum: 0,
          option: 0,
          projected: false,
          carrierLane: false,
          target: null,
          growth: 0,
          alpha: 0,
          blur: 0,
          hold: holdSeconds,
        };
        entries.set(key, e);
      }
      e.y0 = lane.y0;
      e.y1 = lane.y1;
      e.centerY = lane.centerY;
      e.x0 = lane.x0;
      e.x1 = lane.x1;
      e.targetScore = clamp01(lane.score);
      e.momentum = clamp01(lane.momentum);
      e.option = clamp01(lane.option);
      e.projected = Boolean(lane.projected);
      e.carrierLane = Boolean(lane.carrierLane);
      e.target = lane.target ?? null;
      e.projectedPoint = e.projected ? projectedPoint : null;
      e.hold = holdSeconds;
      const k = 1 - Math.exp(-step / targetTau);
      e.score += (e.targetScore - e.score) * k;
      e.growth = Math.min(1, e.growth + step / growIn);
      e.alpha = Math.min(1, e.alpha + step / (growIn * 0.8));
      e.blur = Math.max(0, e.blur - (step / fadeOut) * maxBlur);
    }

    for (const [key, e] of entries) {
      if (seen.has(key)) continue;
      e.targetScore = 0;
      if (e.hold > 0) {
        if (step <= e.hold) {
          e.hold -= step;
        } else {
          const rem = step - e.hold;
          e.hold = 0;
          e.growth = Math.max(0, e.growth - rem / fadeOut);
          e.alpha = Math.max(0, e.alpha - rem / fadeOut);
          e.blur = Math.min(maxBlur, e.blur + (rem / fadeOut) * maxBlur);
        }
      } else {
        e.growth = Math.max(0, e.growth - step / fadeOut);
        e.alpha = Math.max(0, e.alpha - step / fadeOut);
        e.blur = Math.min(maxBlur, e.blur + (step / fadeOut) * maxBlur);
      }
      if (e.alpha <= 0.01 && e.hold <= 0) entries.delete(key);
    }

    for (const p of pulses) p.age += step;
    pulses = pulses.filter((p) => p.age < p.duration);

    const byLane = new Map();
    for (const p of pulses) {
      const list = byLane.get(p.lane) ?? [];
      list.push({
        lane: p.lane,
        fromX: p.fromX,
        toX: p.toX,
        progress: easeOutQuart(p.age / p.duration),
      });
      byLane.set(p.lane, list);
    }

    return [...entries.values()]
      .sort((a, b) => a.index - b.index)
      .map((e) => ({
        key: e.key,
        team: e.team,
        index: e.index,
        y0: e.y0,
        y1: e.y1,
        centerY: e.centerY,
        x0: e.x0,
        x1: e.x1,
        score: clamp01(e.score),
        momentum: e.momentum,
        option: e.option,
        projected: e.projected,
        carrierLane: e.carrierLane,
        target: e.target,
        projectedPoint: e.projected ? e.projectedPoint : null,
        growth: smoothstep(e.growth),
        alpha: clamp01(e.alpha),
        blur: e.blur,
        pulses: byLane.get(e.index) ?? [],
      }));
  }

  function reset() {
    entries.clear();
    pulses = [];
  }

  function setHold(seconds) {
    const next = Number(seconds);
    if (!Number.isFinite(next) || next < 0) return;
    holdSeconds = next;
    // Existing ghosts should respect a shorter memory immediately; a longer
    // setting applies to lanes that are still being updated.
    for (const e of entries.values()) e.hold = Math.min(e.hold, next);
  }

  return {
    updateLive,
    pulse,
    reset,
    setHold,
    get hold() { return holdSeconds; },
    get activeCount() { return entries.size; },
    get pulseCount() { return pulses.length; },
  };
}
