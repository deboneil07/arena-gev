// Substitution choreography (renderer only).
//
// The engine swaps a departing player's identity for the substitute's on the
// "On" event, leaving the agent slot in the same physical place. That is
// correct for ownership and event accuracy, but it makes the change look like
// a shirt-number pop. This module watches the flat render buffer for a
// `(team, shirt)` change in a slot and animates the two bodies the broadcast
// expects to see:
//
//   old shirt  ──walk off──▶  pavilion / bench
//   new shirt  ◀──walk on──   pavilion / bench
//
// It never changes agent positions, event ownership or the 23×6 buffer. The
// renderer simply hides the live body for the short walk-on and draws the
// incoming ghost instead, then releases the body where the ghost finished.

/** Bench / pavilion points, just outside the top touchline (world metres). */
export const PAVILION = [
  { x: -14, y: 36.2 }, // home
  { x: 14, y: 36.2 }, // away
];

export const SUBSTITUTION = {
  pace: 9.0, // metres per second for the ghost walk
  minDuration: 1.4, // seconds; never a jarring blink
  maxDuration: 4.4, // seconds; keeps a long walk from dominating the view
  fadeIn: 0.14, // incoming fade-in fraction of the walk
  fadeOutStart: 0.76, // outgoing fade-out start
};

function clamp01(v) {
  return Math.max(0, Math.min(1, v));
}

function smoothstep(v) {
  const t = clamp01(v);
  return t * t * (3 - 2 * t);
}

function finitePoint(p) {
  return !!p && Number.isFinite(p.x) && Number.isFinite(p.y);
}

/**
 * Eased point along a straight walk. Kept deliberately simple: a soft start
 * and stop reads as a jog, and exact endpoints mean the incoming ghost
 * finishes on the live player's position with no final snap.
 */
export function substitutionPath(from, to, t) {
  const u = smoothstep(t);
  return {
    x: from.x + (to.x - from.x) * u,
    y: from.y + (to.y - from.y) * u,
  };
}

/**
 * Track identity changes in the render buffer.
 *
 * `update(players, dt)` accepts the same snapshot the renderer draws, whose
 * entries expose at least `{ team, shirt, x, y }`. Slots are keyed by their
 * index because the engine's agent vector is stable; a substitution changes
 * only the shirt number in place.
 */
export function createSubstitutionTracker(options = {}) {
  const pavilion = options.pavilion ?? PAVILION;
  const pace = options.pace ?? SUBSTITUTION.pace;
  const minDuration = options.minDuration ?? SUBSTITUTION.minDuration;
  const maxDuration = options.maxDuration ?? SUBSTITUTION.maxDuration;

  let last = new Map();
  let transitions = [];
  let current = new Map();
  let primed = false;

  function refresh(t, dt) {
    const live = current.get(t.slot);
    if (live && live.team === t.team && finitePoint(live)) {
      t.target = { x: live.x, y: live.y };
    }
    const step = Number.isFinite(dt) && dt > 0 ? dt : 0;
    t.age = Math.min(t.duration, t.age + step);
    const p = t.duration > 0 ? t.age / t.duration : 1;
    t.progress = p;
    t.outgoing = substitutionPath(t.from, t.pavilion, p);
    t.incoming = substitutionPath(t.pavilion, t.target, p);
  }

  function update(players, dt) {
    current = new Map();
    players.forEach((p, i) => current.set(i, p));

    // Advance live transitions first, then detect new swaps. A newly spawned
    // ghost therefore starts at progress 0 on the detection frame.
    for (const t of transitions) refresh(t, dt);
    transitions = transitions.filter((t) => t.age < t.duration);

    if (!primed) {
      remember(players);
      primed = true;
      return items();
    }

    players.forEach((p, i) => {
      if (!p || (p.team !== 0 && p.team !== 1)) {
        last.delete(i);
        return;
      }
      const prev = last.get(i);
      if (
        prev &&
        prev.team === p.team &&
        prev.shirt !== p.shirt &&
        prev.shirt > 0 &&
        p.shirt > 0 &&
        finitePoint(prev) &&
        finitePoint(p)
      ) {
        // A second swap in the same slot replaces the old choreography rather
        // than drawing two incoming ghosts.
        transitions = transitions.filter((t) => t.slot !== i);

        const lane = ((i % 5) - 2) * 1.1;
        const bench = {
          x: (pavilion[p.team] ?? pavilion[0]).x + lane,
          y: (pavilion[p.team] ?? pavilion[0]).y,
        };
        const from = { x: prev.x, y: prev.y };
        const distance = Math.hypot(bench.x - from.x, bench.y - from.y);
        const duration = Math.max(
          minDuration,
          Math.min(maxDuration, distance / pace + 0.35),
        );
        transitions.push({
          slot: i,
          team: p.team,
          offShirt: prev.shirt,
          onShirt: p.shirt,
          from,
          pavilion: bench,
          target: { x: p.x, y: p.y },
          age: 0,
          duration,
          progress: 0,
          outgoing: { ...from },
          incoming: { ...bench },
        });
      }
      last.set(i, {
        team: p.team,
        shirt: p.shirt,
        x: p.x,
        y: p.y,
      });
    });

    return items();
  }

  function remember(players) {
    last = new Map();
    players.forEach((p, i) => {
      if (!p || (p.team !== 0 && p.team !== 1)) return;
      last.set(i, { team: p.team, shirt: p.shirt, x: p.x, y: p.y });
    });
  }

  function items() {
    return transitions.map((t) => {
      const p = t.progress;
      return {
        slot: t.slot,
        team: t.team,
        offShirt: t.offShirt,
        onShirt: t.onShirt,
        progress: p,
        duration: t.duration,
        from: { ...t.from },
        pavilion: { ...t.pavilion },
        target: { ...t.target },
        outgoing: {
          x: t.outgoing.x,
          y: t.outgoing.y,
          shirt: t.offShirt,
          team: t.team,
          alpha: 1 - smoothstep((p - SUBSTITUTION.fadeOutStart) / (1 - SUBSTITUTION.fadeOutStart)),
        },
        incoming: {
          x: t.incoming.x,
          y: t.incoming.y,
          shirt: t.onShirt,
          team: t.team,
          alpha: 0.35 + 0.65 * smoothstep(p / SUBSTITUTION.fadeIn),
        },
      };
    });
  }

  function hiddenSlots() {
    return new Set(transitions.map((t) => t.slot));
  }

  function reset() {
    last = new Map();
    transitions = [];
    current = new Map();
    primed = false;
  }

  return {
    update,
    items,
    hiddenSlots,
    reset,
    get activeCount() {
      return transitions.length;
    },
  };
}
