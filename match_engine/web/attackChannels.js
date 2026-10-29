import { clamp01, smoothstep } from './easing.js';

export const ATTACK_CHANNEL = {
  growIn: 0.32,
  targetTau: 0.28,
  fadeOut: 0.5,
  maxBlur: 2.5,
};

/**
 * Live run-channel tracker.
 *
 * Channels are keyed by runner, so the arrow origin tracks the player's live
 * position every frame. The destination eases toward the latest scored run
 * target (8–16 m ahead), which keeps the arrow stable even though the sampled
 * endpoint updates several times a second. `growth` drives the visual
 * extension from a point at the player to the full curved arrow.
 */
export function createAttackChannelTracker(options = {}) {
  const growIn = options.growIn ?? ATTACK_CHANNEL.growIn;
  const targetTau = options.targetTau ?? ATTACK_CHANNEL.targetTau;
  const fadeOut = options.fadeOut ?? ATTACK_CHANNEL.fadeOut;
  const maxBlur = options.maxBlur ?? ATTACK_CHANNEL.maxBlur;
  const entries = new Map();

  function channelKey(channel) {
    if (channel.player) {
      const team = channel.player.team ?? 0;
      const shirt = channel.player.shirt ?? 0;
      return `${team}:${shirt}`;
    }
    return `${channel.from.x.toFixed(1)}:${channel.from.y.toFixed(1)}`;
  }

  function updateLive(dt, computed, resolve) {
    const step = Number.isFinite(dt) && dt > 0 ? dt : 0;
    const seen = new Set();
    for (const channel of computed ?? []) {
      if (!channel || !channel.to) continue;
      const key = channelKey(channel);
      seen.add(key);
      const live = resolve ? resolve(channel) : null;
      const from = live ?? channel.from;
      let e = entries.get(key);
      if (!e) {
        e = {
          key,
          player: channel.player,
          from: { x: from.x, y: from.y },
          to: { x: from.x, y: from.y },
          target: { x: channel.to.x, y: channel.to.y },
          growth: 0,
          alpha: 0,
          blur: 0,
          score: channel.score ?? 0.5,
          curve: channel.curve ?? 0,
          landing: channel.landing ?? null,
        };
        entries.set(key, e);
      }
      e.player = channel.player;
      e.from.x = from.x;
      e.from.y = from.y;
      e.target.x = channel.to.x;
      e.target.y = channel.to.y;
      const k = 1 - Math.exp(-step / targetTau);
      e.to.x += (e.target.x - e.to.x) * k;
      e.to.y += (e.target.y - e.to.y) * k;
      e.score = channel.score ?? e.score;
      e.curve = channel.curve ?? 0;
      e.landing = channel.landing ?? null;
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
      score: e.score,
      curve: e.curve,
      landing: e.landing,
    }));
  }

  function reset() {
    entries.clear();
  }

  return { updateLive, reset, get activeCount() { return entries.size; } };
}