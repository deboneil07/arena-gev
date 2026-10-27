// Whole-match dribble analysis: zone geometry, per-player stats, filters,
// percentiles and rule-based insights.
//
// DOM-free and Canvas-free so the model can be unit-tested from Node. The
// renderer owns the drawing; the panel owns the markup.
//
// Input rows come from `analysis_log.rs` (field order fixed):
//   event: [type, display_secs, match_secs, period, team, player_id,
//           player_name, x, y, end_x|null, end_y|null, outcome]
//   roster: [player_id, name, shirt, position_category]

const PITCH_L = 52.5; // half length, metres
const PITCH_H = 34; // half width, metres

export const ZONE_COLS = 3; // thirds, in the attacker's own frame
export const ZONE_ROWS = 4; // horizontal bands, attacker's left → right

// Field indexes into an event row.
export const EV = Object.freeze({
  TY: 0, T: 1, MT: 2, PERIOD: 3, TEAM: 4, PID: 5,
  NAME: 6, X: 7, Y: 8, EX: 9, EY: 10, OUT: 11,
});

export const TAKE_ON = 3;
export const CARRY = 83;
const SHOT_TYPES = new Set([13, 14, 15, 16]); // miss, post, saved, goal
const PASS_TYPES = new Set([1, 2]);
/** Seconds after a dribble in which a shot counts as "created". */
export const POST_SHOT_WINDOW = 8;

const THIRD_LABELS = ['defensive third', 'middle third', 'final third'];
const BAND_LABELS = ['left flank', 'left half-space', 'right half-space', 'right flank'];

/** Effectiveness bands from the brief (0–100 % success). */
export const EFFECTIVENESS = Object.freeze([
  { min: 90, label: 'Elite' },
  { min: 75, label: 'Very good' },
  { min: 60, label: 'Good' },
  { min: 45, label: 'Average' },
  { min: 30, label: 'Below average' },
  { min: 15, label: 'Poor' },
  { min: 0, label: 'Critical' },
]);

export function effectivenessLabel(successPct) {
  const band = EFFECTIVENESS.find((b) => successPct >= b.min);
  return band ? band.label : 'Critical';
}

/** Match clock as mm:ss from a display-seconds value. */
export function formatClock(secs) {
  const t = Math.max(0, Math.floor(secs));
  return `${String(Math.floor(t / 60)).padStart(2, '0')}:${String(t % 60).padStart(2, '0')}`;
}

/** Attacking-frame transform: home attacks +x, away attacks −x. */
function attackFrame(x, y, team) {
  const dir = team === 1 ? -1 : 1;
  return [x * dir, y * dir];
}

/** Zone id 1..12 for a world point seen by `team` (1 = own third, left band). */
export function zoneOf(x, y, team) {
  const [ax, ay] = attackFrame(x, y, team);
  const col = Math.min(ZONE_COLS - 1, Math.max(0, Math.floor(((ax + PITCH_L) / (PITCH_L * 2)) * ZONE_COLS)));
  const row = Math.min(ZONE_ROWS - 1, Math.max(0, Math.floor(((PITCH_H - ay) / (PITCH_H * 2)) * ZONE_ROWS)));
  return row * ZONE_COLS + col + 1;
}

/** World-space rectangle `{x0, y0, x1, y1}` of a zone as seen by `team`. */
export function zoneWorldRect(zoneId, team) {
  const i = Math.min(11, Math.max(0, zoneId - 1));
  const row = Math.floor(i / ZONE_COLS);
  const col = i % ZONE_COLS;
  const dir = team === 1 ? -1 : 1;
  // Attacking-frame bounds, then mirrored back into world space.
  const ax0 = -PITCH_L + col * ((PITCH_L * 2) / ZONE_COLS);
  const ax1 = ax0 + (PITCH_L * 2) / ZONE_COLS;
  const ay1 = PITCH_H - row * ((PITCH_H * 2) / ZONE_ROWS);
  const ay0 = ay1 - (PITCH_H * 2) / ZONE_ROWS;
  const xs = dir === 1 ? [ax0, ax1] : [-ax1, -ax0];
  const ys = dir === 1 ? [ay0, ay1] : [-ay1, -ay0];
  return { x0: xs[0], y0: ys[0], x1: xs[1], y1: ys[1] };
}

/** Human name of a zone, e.g. "left half-space · final third". */
export function zoneLabel(zoneId) {
  const i = Math.min(11, Math.max(0, zoneId - 1));
  return `${BAND_LABELS[Math.floor(i / ZONE_COLS)]} · ${THIRD_LABELS[i % ZONE_COLS]}`;
}

function emptyZone() {
  return { attempts: 0, successes: 0, gains: 0, gainSamples: 0, shots: 0, keep: 0 };
}

function emptyThird() {
  return { attempts: 0, successes: 0, gains: 0, gainSamples: 0, shots: 0, keep: 0 };
}

/** Fraction of `list` matching `predicate`, or null when empty. */
function rate(list, predicate) {
  if (!list.length) return null;
  let n = 0;
  for (const item of list) if (predicate(item)) n++;
  return n / list.length;
}

function sum(list, get) {
  let total = 0;
  for (const item of list) total += get(item) ?? 0;
  return total;
}

/**
 * Build the whole-match index. Returns read-only views plus memoised
 * stat accessors; everything is derived from the static log, so results
 * never depend on playback position (seek-safe).
 */
export function createDribbleIndex(raw) {
  if (!raw || raw.v !== 1 || !Array.isArray(raw.events)) {
    throw new Error('Unsupported analysis log (expected schema v1)');
  }

  // Chronological order drives post-dribble lookups and the goal log; the
  // engine already sorts, this keeps the UI honest if a future feed does
  // not.
  const sorted = raw.events
    .map((row, i) => ({ row, i }))
    .sort((a, b) => a.row[EV.MT] - b.row[EV.MT] || a.i - b.i);

  const events = sorted.map(({ row, i }) => ({
    i,
    ty: row[EV.TY],
    t: row[EV.T],
    mt: row[EV.MT],
    period: row[EV.PERIOD],
    team: row[EV.TEAM],
    pid: row[EV.PID],
    name: row[EV.NAME],
    x: row[EV.X],
    y: row[EV.Y],
    ex: row[EV.EX],
    ey: row[EV.EY],
    ok: row[EV.OUT] === 1,
  }));

  const roster = [0, 1].map((team) =>
    (raw.roster?.[team] ?? []).map((r) => ({
      pid: r[0], name: r[1] || r[0], shirt: r[2], pos: r[3], team,
    })),
  );

  const rosterByPid = new Map();
  for (const side of roster) for (const p of side) rosterByPid.set(p.pid, p);

  // Goal log drives the game-state filter.
  const goals = events
    .filter((e) => e.ty === 16 && (e.team === 0 || e.team === 1))
    .map((e) => ({ mt: e.mt, team: e.team }));

  const scoreAt = (mt, team) => {
    let for_ = 0;
    let against = 0;
    for (const g of goals) {
      if (g.mt > mt) break;
      if (g.team === team) for_++;
      else against++;
    }
    return { for: for_, against };
  };

  /** 'ahead' | 'level' | 'behind' for `team` at match time `mt`. */
  function gameState(mt, team) {
    const s = scoreAt(mt, team);
    return s.for === s.against ? 'level' : s.for > s.against ? 'ahead' : 'behind';
  }

  // ── Dribble extraction ────────────────────────────────────────
  const byPid = new Map();
  const dribbles = [];
  events.forEach((e, i) => {
    if (e.ty !== TAKE_ON && e.ty !== CARRY) return;
    if (e.team !== 0 && e.team !== 1) return;
    const d = {
      i,
      kind: e.ty === TAKE_ON ? 'takeon' : 'carry',
      ty: e.ty,
      t: e.t,
      mt: e.mt,
      period: e.period,
      team: e.team,
      pid: e.pid,
      name: e.name,
      x: e.x,
      y: e.y,
      ex: e.ex,
      ey: e.ey,
      ok: e.ok,
      zone: zoneOf(e.x, e.y, e.team),
      // Forward gain in metres when the feed carries an end point.
      gain: e.ex == null ? null : (e.ex - e.x) * (e.team === 1 ? -1 : 1),
      shotAfter: null,
      keepAfter: null,
      arrow: null,
    };
    dribbles.push(d);
    if (e.pid) {
      if (!byPid.has(e.pid)) byPid.set(e.pid, []);
      byPid.get(e.pid).push(d);
    }
  });

  // Post-dribble outcome: what the same player did next — scan his whole
  // event list, not just his dribbles, until the window closes. The first
  // event's origin also gives the arrow direction for take-ons the feed
  // records without an end point.
  const REACH_WINDOW = 5;
  const eventsByPid = new Map();
  for (const e of events) {
    if (!e.pid) continue;
    if (!eventsByPid.has(e.pid)) eventsByPid.set(e.pid, []);
    eventsByPid.get(e.pid).push(e);
  }
  for (const [pid, list] of byPid) {
    const own = eventsByPid.get(pid) ?? [];
    for (const d of list) {
      let reachX = null;
      let reachY = null;
      for (const e of own) {
        if (e.mt <= d.mt) continue;
        if (e.mt - d.mt > POST_SHOT_WINDOW) break;
        if (reachX == null && e.mt - d.mt <= REACH_WINDOW) {
          reachX = e.x;
          reachY = e.y;
        }
        if (SHOT_TYPES.has(e.ty)) { d.shotAfter = true; break; }
        if (PASS_TYPES.has(e.ty)) { d.keepAfter = e.ok; break; }
      }
      d.arrow = d.ex != null
        ? { x: d.ex, y: d.ey }
        : (reachX != null ? { x: reachX, y: reachY } : null);
    }
  }

  // ── Filters ───────────────────────────────────────────────────
  /**
   * Apply `{ period: 'all'|1|2, player: pid|'', state: 'all'|'ahead'… }`.
   * `state` is resolved per dribble against that player's team.
   */
  function applyFilters(list, filters = {}) {
    const period = filters.period ?? 'all';
    const state = filters.state ?? 'all';
    let out = list;
    if (period !== 'all') out = out.filter((d) => d.period === Number(period));
    if (filters.player) out = out.filter((d) => d.pid === filters.player);
    if (state !== 'all') out = out.filter((d) => gameState(d.mt, d.team) === state);
    return out;
  }

  // ── Stats ─────────────────────────────────────────────────────
  function statsFor(list) {
    const zones = Array.from({ length: ZONE_COLS * ZONE_ROWS }, emptyZone);
    const thirds = [emptyThird(), emptyThird(), emptyThird()];
    let successes = 0;
    let takeons = 0;
    let takeonWins = 0;
    let lost = 0;
    let shotAfter = 0;
    let keepAfter = 0;
    let gainTotal = 0;
    let gainSamples = 0;

    list.forEach((d) => {
      if (d.ok) successes++;
      if (d.kind === 'takeon') {
        takeons++;
        if (d.ok) takeonWins++;
      }
      if (!d.ok) lost++;
      if (d.shotAfter) shotAfter++;
      if (d.keepAfter) keepAfter++;
      if (d.gain != null) {
        gainTotal += d.gain;
        gainSamples++;
      }
      const z = zones[d.zone - 1];
      z.attempts++;
      if (d.ok) z.successes++;
      if (d.gain != null) { z.gains += d.gain; z.gainSamples++; }
      if (d.shotAfter) z.shots++;
      if (d.keepAfter) z.keep++;
      const third = thirds[(d.zone - 1) % ZONE_COLS];
      third.attempts++;
      if (d.ok) third.successes++;
      if (d.gain != null) { third.gains += d.gain; third.gainSamples++; }
      if (d.shotAfter) third.shots++;
      if (d.keepAfter) third.keep++;
    });

    const attempts = list.length;
    const finish = (z) => ({
      ...z,
      success: z.attempts ? z.successes / z.attempts : null,
      avgGain: z.gainSamples ? z.gains / z.gainSamples : null,
    });

    return {
      attempts,
      successes,
      success: attempts ? successes / attempts : null,
      takeons,
      takeonWins,
      lost,
      shotAfter,
      shotRate: attempts ? shotAfter / attempts : null,
      keepAfter,
      keepRate: attempts ? keepAfter / attempts : null,
      gainTotal,
      gainSamples,
      avgGain: gainSamples ? gainTotal / gainSamples : null,
      zones: zones.map(finish),
      thirds: thirds.map(finish),
      dribbles: list,
    };
  }

  const statCache = new Map();
  function cachedStats(list, key) {
    if (!statCache.has(key)) statCache.set(key, statsFor(list));
    return statCache.get(key);
  }

  function playerStats(pid, filters = {}) {
    const key = `p:${pid}:${filters.period ?? 'all'}:${filters.state ?? 'all'}`;
    const base = byPid.get(pid) ?? [];
    return cachedStats(applyFilters(base, filters), key);
  }

  function teamStats(team, filters = {}) {
    const key = `t:${team}:${filters.period ?? 'all'}:${filters.state ?? 'all'}:${filters.player ?? ''}`;
    const base = dribbles.filter((d) => d.team === team);
    return cachedStats(applyFilters(base, filters), key);
  }

  // ── Percentiles vs team-mates (same match, ≥2 dribbles) ───────
  function percentileBenchmarks(team, filters = {}) {
    const mates = roster[team]
      .map((p) => ({ p, stats: playerStats(p.pid, filters) }))
      .filter((m) => m.stats.attempts >= 2);
    return mates;
  }

  function percentileOf(mates, get, value) {
    if (value == null || mates.length < 3) return null;
    const values = mates.map(get).filter((v) => v != null);
    if (values.length < 3) return null;
    const lower = values.filter((v) => v < value).length;
    return Math.round((lower / values.length) * 100);
  }

  function playerPercentiles(pid, filters = {}) {
    const entry = rosterByPid.get(pid);
    if (!entry) return {};
    const mates = percentileBenchmarks(entry.team, filters);
    const stats = playerStats(pid, filters);
    if (stats.attempts === 0) {
      // No dribbles: nothing to rank, and "0th percentile" would mislead.
      return { attempts: null, success: null, gain: null, shotRate: null, mateCount: mates.length };
    }
    return {
      attempts: percentileOf(mates, (m) => m.stats.attempts, stats.attempts),
      success: percentileOf(mates, (m) => m.stats.success, stats.success),
      // Only players with usable end-point data take part in the gain table.
      gain: percentileOf(
        mates,
        (m) => (m.stats.gainSamples ? m.stats.gainTotal : null),
        stats.gainSamples ? stats.gainTotal : null,
      ),
      shotRate: percentileOf(mates, (m) => m.stats.shotRate, stats.shotRate),
      mateCount: mates.length,
    };
  }

  /** 1–5 stars from a 0–100 percentile (45 → ★★★☆☆, 82 → ★★★★☆, 91 → ★★★★★). */
  function stars(pct) {
    if (pct == null) return 0;
    if (pct >= 90) return 5;
    if (pct >= 60) return 4;
    if (pct >= 40) return 3;
    if (pct >= 20) return 2;
    return 1;
  }

  // ── Insights ──────────────────────────────────────────────────
  function insights(stats, label = 'this player') {
    const out = [];
    const pct = (n, d) => `${Math.round((n / d) * 100)}%`;

    // Strength: a third of the pitch with real volume and a clear edge.
    if (stats.attempts >= 4) {
      const ranked = stats.thirds
        .map((third, i) => ({ ...third, i }))
        .filter((third) => third.attempts >= 3 && third.success != null)
        .sort((a, b) => b.success - a.success);
      const best = ranked[0];
      if (best && best.success >= 0.7 && best.success >= (stats.success ?? 0) + 0.05) {
        out.push({
          kind: 'strength',
          title: `Retains the ball in the ${THIRD_LABELS[best.i]}`,
          body: `${pct(best.successes, best.attempts)} success from ${best.attempts} dribbles there — well above ${pct(stats.successes, Math.max(1, stats.attempts))} overall.`,
        });
      }
    }

    // Concern: the final third swallowing attempts at a poor rate.
    const finalThird = stats.thirds[2];
    if (finalThird && finalThird.attempts >= 5 && finalThird.success != null && finalThird.success < 0.6) {
      out.push({
        kind: 'concern',
        title: 'Struggles to beat markers in the final third',
        body: `${pct(finalThird.successes, finalThird.attempts)} success on ${finalThird.attempts} attempts. Look for combination play instead of solo runs.`,
      });
    }

    // Pattern: one zone carrying the volume.
    if (stats.attempts >= 5) {
      const busiest = stats.zones
        .map((z, i) => ({ ...z, i }))
        .sort((a, b) => b.attempts - a.attempts)[0];
      if (busiest && busiest.attempts >= 3 && busiest.attempts / stats.attempts >= 0.3) {
        out.push({
          kind: 'pattern',
          title: `Favours the ${zoneLabel(busiest.i + 1)}`,
          body: `${busiest.attempts} of ${stats.attempts} dribbles (${pct(busiest.attempts, stats.attempts)}) start there.`,
        });
      }
    }

    // Opportunity: a zone that works but is barely used.
    if (stats.attempts >= 4) {
      const underused = stats.zones
        .map((z, i) => ({ ...z, i }))
        .filter((z) => z.attempts >= 2 && z.attempts / stats.attempts <= 0.15 && (z.success ?? 0) >= 0.75)
        .sort((a, b) => b.success - a.success)[0];
      if (underused) {
        out.push({
          kind: 'opportunity',
          title: `Underused: the ${zoneLabel(underused.i + 1)}`,
          body: `${pct(underused.successes, underused.attempts)} success from only ${underused.attempts} attempts — room to attack it more.`,
        });
      }
    }

    // Late-match drop-off: last 20 minutes versus everything before.
    if (stats.attempts >= 8) {
      const late = stats.dribbles.filter((d) => d.t >= 70 * 60);
      const early = stats.dribbles.filter((d) => d.t < 70 * 60);
      if (late.length >= 4 && early.length >= 4) {
        const lateRate = rate(late, (d) => d.ok);
        const earlyRate = rate(early, (d) => d.ok);
        if (lateRate != null && earlyRate != null && earlyRate - lateRate >= 0.12) {
          out.push({
            kind: 'pattern',
            title: 'More conservative after the 70th minute',
            body: `Success drops from ${pct(early.filter((d) => d.ok).length, early.length)} to ${pct(late.filter((d) => d.ok).length, late.length)} in the final 20 minutes — fatigue or instruction.`,
          });
        }
      }
    }

    if (!out.length && stats.attempts === 0) {
      out.push({
        kind: 'empty',
        title: `No dribbles recorded for ${label}`,
        body: 'Switch the half or game-state filter, or pick another player.',
      });
    }
    return out;
  }

  function rosterFor(team) {
    return roster[team] ?? [];
  }

  /** Resolve a clicked pitch position to a dribble of the selection, if any. */
  function dribbleNear(x, y, team, radius = 4) {
    let best = null;
    let bestD = radius * radius;
    for (const d of dribbles) {
      if (team != null && d.team !== team) continue;
      const dx = d.x - x;
      const dy = d.y - y;
      const dist = dx * dx + dy * dy;
      if (dist <= bestD) { bestD = dist; best = d; }
    }
    return best;
  }

  return {
    events,
    dribbles,
    roster,
    rosterByPid,
    goals,
    gameState,
    scoreAt,
    applyFilters,
    statsFor,
    playerStats,
    teamStats,
    playerPercentiles,
    stars,
    insights,
    rosterFor,
    dribbleNear,
    /** Player entry for a (team, shirt) render-buffer pair. */
    playerByKey(team, shirt) {
      return roster[team]?.find((p) => p.shirt === shirt) ?? null;
    },
    playerById(pid) {
      return rosterByPid.get(pid) ?? null;
    },
    dribblesOf(pid) {
      return byPid.get(pid) ?? [];
    },
    /** Whole-match dribbles of one team, optionally filtered. */
    teamDribbles(team, filters) {
      return applyFilters(dribbles.filter((d) => d.team === team), filters);
    },
  };
}
