// Event-synced scripted sequences (renderer only).
//
// The engine owns ground truth (who scored, who saved, who walked off); the
// renderer adds the broadcast theatre the feed implies but never logs:
// a goal mob converging on the scorer, a parry burst, woodwork ping, an
// offside flag freeze, a card banner. All ghosts: agent positions, event
// ownership and the 23×6 buffer are untouched, every sequence is eased with
// smoothstep, capped in duration, and cancelled on seek (reset()).
//
// Triggering keys off `engine.get_recent_event_*` deltas observed in app.js:
// this module only needs `{ type, team, playerIdx, x, y }` sightings plus the
// live player/ball snapshots to retarget ghosts as bodies move.

function clamp01(v) {
  return Math.max(0, Math.min(1, v));
}

function smoothstep(v) {
  const t = clamp01(v);
  return t * t * (3 - 2 * t);
}

export const SEQUENCE = Object.freeze({
  goalDuration: 3.0, // scorer run + mob converge
  mobCount: 3, // teammates converging (scorer excluded)
  burstDuration: 0.9, // save parry puff / post ping
  flagDuration: 1.2, // offside flag freeze
  bannerDuration: 2.6, // card / penalty / VAR banner
});

/**
 * Goal celebration: the scorer sprints toward the corner arc while up to
 * three nearest teammates converge on him. Ghosts only — the live bodies
 * keep their simulated positions; the tracker hides nothing.
 */
export function createCelebrationTracker(options = {}) {
  const duration = options.duration ?? SEQUENCE.goalDuration;
  const mobCount = options.mobCount ?? SEQUENCE.mobCount;
  let active = null; // { scorer, mob:[{from}], age }

  function trigger(scorer, teammates) {
    if (!scorer || !Number.isFinite(scorer.x) || !Number.isFinite(scorer.y)) return;
    const corner = {
      x: Math.sign(scorer.x || 1) * 44,
      y: Math.sign(scorer.y || 1) * 28,
    };
    const mob = (teammates ?? [])
      .filter((p) => p && p !== scorer && Number.isFinite(p.x) && Number.isFinite(p.y))
      .map((p) => ({ from: { x: p.x, y: p.y }, mate: p }))
      .slice(0, mobCount);
    active = { scorer, mob, corner, age: 0 };
  }

  function update(players, ball, dt) {
    if (!active) return null;
    const step = Number.isFinite(dt) && dt > 0 ? dt : 0;
    active.age = Math.min(duration, active.age + step);
    const p = active.age / duration;
    // Refresh endpoints so ghosts track the (mostly still) live bodies.
    const s = active.scorer;
    const head = { x: s.x, y: s.y };
    const dest = active.corner;
    const run = {
      x: head.x + (dest.x - head.x) * smoothstep(p),
      y: head.y + (dest.y - head.y) * smoothstep(p),
      shirt: s.shirt,
      team: s.team,
      alpha: 1 - smoothstep((p - 0.7) / 0.3),
    };
    const mob = active.mob.map((m) => {
      const mx = m.mate.x;
      const my = m.mate.y;
      return {
        x: m.from.x + (mx - m.from.x) * smoothstep(p),
        y: m.from.y + (my - m.from.y) * smoothstep(p),
        shirt: m.mate.shirt,
        team: m.mate.team,
        alpha: 0.85 * (1 - smoothstep((p - 0.75) / 0.25)),
      };
    });
    const done = active.age >= duration;
    const out = { run, mob, progress: p };
    if (done) active = null;
    return out;
  }

  function reset() {
    active = null;
  }

  return { trigger, update, reset, get active() { return !!active; } };
}

/**
 * One-shot pitch effects: save puff, woodwork ping, offside flag freeze,
 * banner captions. Each effect is `{ kind, x, y, age, duration }`; the
 * renderer draws them in overlays.js and they expire on their own.
 */
export function createFxTracker(options = {}) {
  const durations = {
    save: options.burstDuration ?? SEQUENCE.burstDuration,
    post: (options.burstDuration ?? SEQUENCE.burstDuration) * 1.3,
    flag: options.flagDuration ?? SEQUENCE.flagDuration,
    banner: options.bannerDuration ?? SEQUENCE.bannerDuration,
  };
  let effects = [];
  let seq = 0;

  function spawn(kind, x, y, extra = {}) {
    if (!Number.isFinite(x) || !Number.isFinite(y)) return;
    effects = effects.filter((e) => e.kind !== kind || kind === 'banner');
    effects.push({
      id: ++seq,
      kind,
      x,
      y,
      age: 0,
      duration: durations[kind] ?? 1.0,
      ...extra,
    });
  }

  function update(dt) {
    const step = Number.isFinite(dt) && dt > 0 ? dt : 0;
    for (const e of effects) e.age = Math.min(e.duration, e.age + step);
    effects = effects.filter((e) => e.age < e.duration);
    return effects.map((e) => ({ ...e, progress: e.duration > 0 ? e.age / e.duration : 1 }));
  }

  function reset() {
    effects = [];
  }

  return { spawn, update, reset };
}
