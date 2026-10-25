// Pure tactical-analysis geometry for the God's Eye View renderer.
//
// Nothing in here touches the DOM, Canvas or WASM: the renderer passes plain
// player snapshots in and gets overlay geometry out. Keeping it pure means the
// thresholds and the maths can be exercised from Node (`analysis.test.mjs`)
// and tuned against real fixtures without a browser.
//
// Coordinate convention: engine metres, origin at the centre spot. Home
// attacks +x (toward the away goal), away attacks −x; the frame is canonical
// and stable across half-time.

export const ROLE = Object.freeze({
  UNKNOWN: 0,
  GK: 1,
  DEF: 2,
  MID: 3,
  FWD: 4,
});

/**
 * Tuning constants. These are intentionally in one place so they can be
 * sampled against the bundled fixtures and adjusted until the overlays are
 * useful but not permanent.
 */
export const ANALYSIS = Object.freeze({
  // Midfield unit: compact enough to read as one bank, wide enough to be a
  // line rather than a cluster.
  midfieldMaxDeviation: 11.0, // max |x − mean x|, metres
  midfieldMaxSpread: 18.0, // max x − min x, metres
  midfieldMinSpan: 8.0, // lateral extent, metres
  midfieldMaxGap: 28.0, // largest gap between adjacent midfielders, metres
  midfieldBankSplit: 10.0, // depth gap that separates two banks, metres
  midfieldMaxBanks: 2,

  // Threat scoring.
  threatMinProgress: 10.0, // metres past halfway
  threatMinScore: 0.52,
  threatMinSpace: 2.2, // metres from the nearest opponent
  threatOffsideMargin: 0.5, // metres of tolerance on the offside line

  // Danger zone: small, legible, and anchored to real players.
  // The caps trim the rare polygon that spans most of the penalty area.
  zoneMinArea: 20.0, // m²
  zoneMaxArea: 360.0, // m² for a 3–6 vertex polygon
  zoneMaxEdge: 34.0, // metres
  zoneMaxVertices: 6,
  zoneTriangleMaxArea: 280.0, // fallback triangle stays tighter
  zoneTriangleMaxEdge: 30.0,
  zoneCoverDistance: 22.0, // max distance to each flanking marker, metres

  // Pass lane. Real passing lanes are tight; 2 m keeps the overlay honest
  // without making it effectively disappear.
  laneClearance: 2.0, // metres either side of the ball → threat segment
  laneMinBallDistance: 7.0,
  laneMaxBallDistance: 50.0,
  laneMinThreatScore: 0.55,

  // Pass fan: at most this many candidate corridors, sampled this finely.
  fanMaxOptions: 3,
  fanSampleStep: 0.5, // metres along the corridor
  fanInterceptMargin: 0.35, // seconds the ball must beat a defender by
  fanDefenderAccel: 4.5, // m/s² assumed for a reacting defender

  // Space control: local grid around the threat, cells the attacker wins by
  // more than this margin (seconds) over the nearest defender.
  spaceGridHalf: 4.0, // metres each way
  spaceGridStep: 2.0, // metres
  spaceWinMargin: 0.5, // seconds

  // xT surface: transparent heuristic, calibrated on the bundled corpus —
  // labelled "model estimate" in the UI, never claimed as measured xT.
  xtGridCols: 12,
  xtGridRows: 8,
});

const SHAPE_LABELS = Object.freeze({
  '4-4-2': '4-4-2',
  '4-3-3': '4-3-3',
  '4-5-1': '4-2-3-1',
  '3-5-2': '3-5-2',
  '5-3-2': '5-3-2',
  '4-1-1': '4-1-4-1',
});

export function clamp01(value) {
  return Math.min(1, Math.max(0, value));
}

export function distance(a, b) {
  const dx = a.x - b.x;
  const dy = a.y - b.y;
  return Math.hypot(dx, dy);
}

/** Human-readable shape from the starting XI's role counts, e.g. "4-3-3". */
export function formationLabel(players) {
  let def = 0;
  let mid = 0;
  let fwd = 0;
  for (const p of players) {
    if (p.role === ROLE.DEF) def++;
    else if (p.role === ROLE.MID) mid++;
    else if (p.role === ROLE.FWD) fwd++;
  }
  const key = `${def}-${mid}-${fwd}`;
  return SHAPE_LABELS[key] ?? key;
}

/**
 * Detect settled midfield banks.
 *
 * A bank is shown when at least two midfielders hold a compact vertical line:
 * their x spread is small relative to the team shape, they span enough of the
 * width to be a real line, and nobody has broken away by a large margin.
 *
 * A 4-2-3-1 keeps two distinct banks (the double pivot and the three ahead),
 * so the midfielders are clustered by depth first. A single pressing
 * midfielder then drops out of the bank instead of hiding the whole unit.
 *
 * @param {Array<{x:number,y:number,role:number}>} teamPlayers
 * @returns {Array<{points:Array, tightness:number, meanX:number, spanY:number}>}
 */
export function detectMidfieldBanks(teamPlayers) {
  const mids = teamPlayers
    .filter((p) => p.role === ROLE.MID)
    .slice()
    .sort((a, b) => a.x - b.x);
  if (mids.length < 2) return [];

  // Split on a real change of depth. A 4-3-3's single pivot (six metres
  // behind the two eights) stays one bank; a 4-2-3-1's fifteen-metre split
  // becomes two.
  const groups = [];
  let current = [mids[0]];
  for (let i = 1; i < mids.length; i++) {
    if (mids[i].x - mids[i - 1].x > ANALYSIS.midfieldBankSplit) {
      groups.push(current);
      current = [];
    }
    current.push(mids[i]);
  }
  groups.push(current);

  const banks = [];
  for (const group of groups) {
    const bank = buildMidfieldBank(group);
    if (bank) banks.push(bank);
  }
  return banks.slice(0, ANALYSIS.midfieldMaxBanks);
}

function buildMidfieldBank(mids) {
  if (mids.length < 2) return null;

  let sumX = 0;
  let minX = Infinity;
  let maxX = -Infinity;
  for (const p of mids) {
    sumX += p.x;
    if (p.x < minX) minX = p.x;
    if (p.x > maxX) maxX = p.x;
  }
  const meanX = sumX / mids.length;
  let maxDeviation = 0;
  for (const p of mids) maxDeviation = Math.max(maxDeviation, Math.abs(p.x - meanX));
  const spread = maxX - minX;

  const points = mids.slice().sort((a, b) => b.y - a.y);
  const spanY = points[0].y - points[points.length - 1].y;
  let maxGap = 0;
  for (let i = 1; i < points.length; i++) {
    maxGap = Math.max(maxGap, Math.abs(points[i - 1].y - points[i].y));
  }

  if (
    maxDeviation > ANALYSIS.midfieldMaxDeviation ||
    spread > ANALYSIS.midfieldMaxSpread ||
    spanY < ANALYSIS.midfieldMinSpan ||
    maxGap > ANALYSIS.midfieldMaxGap
  ) {
    return null;
  }

  return {
    points,
    tightness: clamp01(1 - maxDeviation / ANALYSIS.midfieldMaxDeviation),
    meanX,
    spanY,
  };
}

/**
 * Backwards-compatible single-bank helper: returns the largest bank, or null.
 * The renderer uses [`detectMidfieldBanks`] so a double pivot and a No.10
 * line are drawn separately.
 */
export function detectMidfieldBank(teamPlayers) {
  const banks = detectMidfieldBanks(teamPlayers);
  if (banks.length === 0) return null;
  return banks.reduce((best, bank) =>
    bank.points.length > best.points.length ? bank : best,
  );
}

/** Forwards and midfielders, or the two most advanced outfield players. */
export function attackingCandidates(teamPlayers, dir) {
  const attacking = teamPlayers.filter(
    (p) => p.role === ROLE.FWD || p.role === ROLE.MID,
  );
  if (attacking.length > 0) {
    return attacking.sort((a, b) => b.x * dir - a.x * dir);
  }
  return teamPlayers
    .filter((p) => p.role !== ROLE.GK)
    .sort((a, b) => b.x * dir - a.x * dir)
    .slice(0, 2);
}

/**
 * Pick the two markers that define the threat's coverage triangle.
 *
 * Choosing simply the two nearest opponents often yields a thin sliver when
 * both are goal-side of the attacker. Bracketing the shooting lane instead
 * produces a triangle that represents the space he can actually attack, and
 * the two vertices are still real players. When only one flank is marked,
 * fall back to the two nearest opponents so the zone still tracks play.
 * Returns null with fewer than two markers inside `maxDistance`.
 */
export function selectCoverPair(player, goal, cover, maxDistance = ANALYSIS.zoneCoverDistance) {
  const gx = goal.x - player.x;
  const gy = goal.y - player.y;
  const goalLength = Math.hypot(gx, gy) || 1;
  let left = null;
  let right = null;
  let leftDistance = Infinity;
  let rightDistance = Infinity;
  let nearest = null;
  let second = null;
  let nearestDistance = Infinity;
  let secondDistance = Infinity;

  for (const opponent of cover) {
    const dx = opponent.x - player.x;
    const dy = opponent.y - player.y;
    const distanceToOpponent = Math.hypot(dx, dy);
    if (distanceToOpponent < 1e-6 || distanceToOpponent > maxDistance) continue;

    // The coverage area belongs in front of the attacker, not behind him.
    // Allow a small tolerance so a shoulder-to-shoulder marker still counts.
    const goalSide = (dx * gx + dy * gy) / goalLength;
    if (goalSide < -2.0) continue;

    if (distanceToOpponent < nearestDistance) {
      second = nearest;
      secondDistance = nearestDistance;
      nearest = opponent;
      nearestDistance = distanceToOpponent;
    } else if (distanceToOpponent < secondDistance) {
      second = opponent;
      secondDistance = distanceToOpponent;
    }

    // Sine of the angle between the goal direction and this marker. Values
    // near zero mean the marker is almost directly in front or behind, which
    // would make the triangle degenerate.
    const side = (gx * dy - gy * dx) / (goalLength * distanceToOpponent);
    if (Math.abs(side) < 0.08) continue;

    if (side > 0) {
      if (distanceToOpponent < leftDistance) {
        left = opponent;
        leftDistance = distanceToOpponent;
      }
    } else if (distanceToOpponent < rightDistance) {
      right = opponent;
      rightDistance = distanceToOpponent;
    }
  }

  if (left && right) return [left, right];
  return nearest && second ? [nearest, second] : null;
}

/**
 * Pick the real players that bound the threat's attack zone.
 *
 * The zone should not always be a triangle. We divide the goal-side arc into
 * five angular sectors and keep the nearest marker in each; a back four in
 * front of the carrier then produces a quadrilateral or pentagon. Markers
 * behind the threat are ignored so the threat remains the near vertex (and the
 * polygon never wraps around behind play).
 *
 * Returns 2–5 markers, nearest first within each sector.
 */
export function selectCoverMarkers(
  player,
  goal,
  cover,
  maxDistance = ANALYSIS.zoneCoverDistance,
) {
  const gx = goal.x - player.x;
  const gy = goal.y - player.y;
  const goalLength = Math.hypot(gx, gy) || 1;
  const sectors = new Array(5).fill(null);
  const sectorDistance = new Array(5).fill(Infinity);
  const nearest = [];
  const seen = new Set();

  const remember = (opponent) => {
    const key = `${opponent.x.toFixed(3)}:${opponent.y.toFixed(3)}:${opponent.team ?? ''}:${opponent.shirt ?? ''}`;
    if (seen.has(key)) return;
    seen.add(key);
    nearest.push(opponent);
  };

  for (const opponent of cover) {
    const dx = opponent.x - player.x;
    const dy = opponent.y - player.y;
    const distanceToOpponent = Math.hypot(dx, dy);
    if (distanceToOpponent < 1e-6 || distanceToOpponent > maxDistance) continue;

    const goalSide = (dx * gx + dy * gy) / goalLength;
    if (goalSide <= 0.0) continue;

    // Signed angle against the goal direction, in radians (−π/2…π/2).
    const side = (gx * dy - gy * dx) / (goalLength * distanceToOpponent);
    const angle = Math.asin(Math.min(1, Math.max(-1, side)));
    const index = Math.min(
      4,
      Math.max(0, Math.floor(((angle + Math.PI / 2) / Math.PI) * 5)),
    );
    if (distanceToOpponent < sectorDistance[index]) {
      sectorDistance[index] = distanceToOpponent;
      sectors[index] = opponent;
    }
    remember(opponent);
  }

  const markers = [];
  const added = new Set();
  for (const marker of sectors) {
    if (!marker) continue;
    const key = `${marker.x.toFixed(3)}:${marker.y.toFixed(3)}:${marker.team ?? ''}:${marker.shirt ?? ''}`;
    if (added.has(key)) continue;
    added.add(key);
    markers.push(marker);
  }

  // A single occupied sector still needs a second point for a polygon; use
  // the nearest overall markers as the old pair fallback did.
  nearest.sort(
    (a, b) => distance(player, a) - distance(player, b),
  );
  for (const marker of nearest) {
    if (markers.length >= 2) break;
    const key = `${marker.x.toFixed(3)}:${marker.y.toFixed(3)}:${marker.team ?? ''}:${marker.shirt ?? ''}`;
    if (added.has(key)) continue;
    added.add(key);
    markers.push(marker);
  }

  return markers.slice(0, 5);
}

function pointSegmentDistance(p, a, b) {
  const abx = b.x - a.x;
  const aby = b.y - a.y;
  const apx = p.x - a.x;
  const apy = p.y - a.y;
  const abLenSq = abx * abx + aby * aby;
  if (abLenSq < 1e-9) return Math.hypot(apx, apy);
  const t = Math.max(0, Math.min(1, (apx * abx + apy * aby) / abLenSq));
  return Math.hypot(apx - abx * t, apy - aby * t);
}

/** True when no opponent is within `clearance` metres of the pass segment. */
export function passLaneClear(from, to, opponents, clearance = ANALYSIS.laneClearance) {
  for (const o of opponents) {
    if (pointSegmentDistance(o, from, to) < clearance) return false;
  }
  return true;
}

/**
 * Score every viable attacker and return them best-first.
 *
 * `cover` is the set of players eligible to become the danger triangle's
 * vertices — normally every outfield opponent, so the triangle hugs the
 * attacker and his immediate markers. Each entry is self-contained so the UI
 * can lock onto one and draw the triangle without re-running the maths.
 */
export function evaluateThreats({
  attackers,
  cover,
  opponents,
  ball,
  dir,
  offsideX = NaN,
}) {
  if (!attackers?.length || !cover || cover.length < 2 || !ball) return [];

  const goal = { x: dir * 52.5, y: 0 };
  const ranked = [];

  for (const player of attackers) {
    if (!player || !Number.isFinite(player.x) || !Number.isFinite(player.y)) continue;

    const progress = player.x * dir;
    if (progress < ANALYSIS.threatMinProgress) continue;

    if (Number.isFinite(offsideX)) {
      const beyond =
        dir > 0
          ? player.x > offsideX + ANALYSIS.threatOffsideMargin
          : player.x < offsideX - ANALYSIS.threatOffsideMargin;
      if (beyond) continue;
    }

    let nearestOpponent = Infinity;
    for (const o of opponents) {
      const d = distance(player, o);
      if (d < nearestOpponent) nearestOpponent = d;
    }
    if (!Number.isFinite(nearestOpponent)) nearestOpponent = 30;
    if (nearestOpponent < ANALYSIS.threatMinSpace) continue;

    const goalDistance = distance(player, goal);
    const goalProximity = clamp01(1 - goalDistance / 45);
    const space = clamp01(nearestOpponent / 12);
    const centrality = clamp01(1 - Math.abs(player.y) / 30);
    const ballDistance = distance(player, ball);
    const ballLink = clamp01(1 - ballDistance / 40);
    const run = clamp01((player.speed || 0) / 8);

    const score =
      0.42 * goalProximity +
      0.28 * space +
      0.12 * centrality +
      0.1 * ballLink +
      0.08 * run;

    if (score < ANALYSIS.threatMinScore) continue;

    const coverMarkers = selectCoverMarkers(player, goal, cover);
    const coverPair = selectCoverPair(player, goal, cover);
    const laneClear =
      ballDistance >= ANALYSIS.laneMinBallDistance &&
      ballDistance <= ANALYSIS.laneMaxBallDistance &&
      score >= ANALYSIS.laneMinThreatScore &&
      passLaneClear(ball, player, opponents);

    ranked.push({
      player,
      score,
      nearestOpponent,
      goalDistance,
      ballDistance,
      laneClear,
      goal,
      cover: coverMarkers,
      coverPair,
    });
  }

  ranked.sort((a, b) => b.score - a.score);
  return ranked;
}

function cross(o, a, b) {
  return (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
}

/** Convex hull (monotone chain), duplicate points removed. */
export function convexHull(points) {
  const unique = [];
  for (const p of points) {
    if (!unique.some((q) => Math.abs(q.x - p.x) < 1e-6 && Math.abs(q.y - p.y) < 1e-6)) {
      unique.push({ x: p.x, y: p.y });
    }
  }
  if (unique.length <= 2) return unique;
  unique.sort((a, b) => (a.x === b.x ? a.y - b.y : a.x - b.x));

  const lower = [];
  for (const p of unique) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) {
      lower.pop();
    }
    lower.push(p);
  }
  const upper = [];
  for (let i = unique.length - 1; i >= 0; i--) {
    const p = unique[i];
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) {
      upper.pop();
    }
    upper.push(p);
  }
  lower.pop();
  upper.pop();
  return lower.concat(upper);
}

/** Shoelace area in square metres (always positive). */
export function polygonArea(points) {
  if (!points || points.length < 3) return 0;
  let sum = 0;
  for (let i = 0; i < points.length; i++) {
    const a = points[i];
    const b = points[(i + 1) % points.length];
    sum += a.x * b.y - b.x * a.y;
  }
  return Math.abs(sum) * 0.5;
}

function measureZone(hull, maxArea, maxEdge) {
  if (hull.length < 3) return null;
  const area = polygonArea(hull);
  if (area < ANALYSIS.zoneMinArea || area > maxArea) return null;

  let edge = 0;
  for (let i = 0; i < hull.length; i++) {
    edge = Math.max(edge, distance(hull[i], hull[(i + 1) % hull.length]));
  }
  if (edge > maxEdge) return null;
  return { points: hull, area, maxEdge: edge, vertices: hull.length };
}

/**
 * Build the compact attack zone around a threat.
 *
 * The preferred shape is the convex hull of the threat plus the eligible
 * goal-side markers, giving a 3–6 vertex polygon that follows the real
 * blocking envelope. If that grows too large or degenerates, fall back to the
 * classic triangle (threat + the two bracketing markers). A triangle is now
 * the exception, not the default.
 */
export function buildDangerZone(threat) {
  if (!threat?.player || !threat.cover || threat.cover.length < 2) return null;

  const markers = threat.cover.slice(0, ANALYSIS.zoneMaxVertices - 1);
  const polygon = measureZone(
    convexHull([threat.player, ...markers]),
    ANALYSIS.zoneMaxArea,
    ANALYSIS.zoneMaxEdge,
  );
  if (polygon) return polygon;

  // Fallback triangle: the old pair when evaluateThreats supplied one, or the
  // first two markers for callers that construct a threat directly.
  const pair =
    threat.coverPair?.length === 2 ? threat.coverPair : threat.cover.slice(0, 2);
  const triangle = measureZone(
    convexHull([threat.player, ...pair]),
    ANALYSIS.zoneTriangleMaxArea,
    ANALYSIS.zoneTriangleMaxEdge,
  );
  if (triangle && triangle.points.length === 3) return triangle;
  return null;
}

/** Convenience: true when the threat is an actual on-ball player. */
export function isOnBall(threat, ball, radius = 2.5) {
  return !!threat?.player && distance(threat.player, ball) <= radius;
}

// ── Expected-threat surface (transparent heuristic) ──────────────
//
// A coarse 12×8 grid over the pitch: value peaks in the central final
// third and decays with distance from goal and the touchlines. It is a
// *model estimate* for overlay weighting — labelled as such in the UI and
// never presented as measured xT. Calibrate the peak/edges against the
// bundled event corpus (Brier/log-loss on next-goal labels).
export function xtAt(x, y, dir) {
  const px = x * dir; // progress toward the attacked goal
  const goalDist = Math.hypot(52.5 - px, y);
  const proximity = clamp01(1 - goalDist / 60);
  const centrality = clamp01(1 - Math.abs(y) / 34);
  const progress = clamp01((px + 10) / 62.5);
  return clamp01(0.55 * proximity + 0.25 * centrality + 0.2 * progress);
}

/** Value gained by moving the ball from `from` to `to` (both `{x, y}`). */
export function xtGain(from, to, dir) {
  return xtAt(to.x, to.y, dir) - xtAt(from.x, from.y, dir);
}

/** Shot value heuristic (0.02–0.65) by distance/angle — a model estimate. */
export function shotXg(x, y, dir) {
  const px = x * dir;
  const d = Math.hypot(52.5 - px, y);
  if (d > 35) return 0.02;
  const angle = Math.abs(Math.atan2(Math.abs(y), Math.max(1, 52.5 - px)));
  const base = clamp01(1 - d / 32);
  return Number(clamp01(0.08 + 0.55 * base * (0.5 + 0.5 * Math.cos(angle))).toFixed(3));
}

/** Predict a player's position `dt` seconds ahead from velocity. */
export function predictPosition(p, dt) {
  return {
    x: p.x + (p.vx || 0) * dt,
    y: p.y + (p.vy || 0) * dt,
  };
}

/** Stop-to-stop time for `dist` metres at caps (mirrors the engine). */
export function arrivalTime(dist, maxSpeed = 8.0, maxAccel = 6.0) {
  if (dist <= 0) return 0;
  const dAccel = (maxSpeed * maxSpeed) / (2 * maxAccel);
  if (dist <= 2 * dAccel) return 2 * Math.sqrt(dist / maxAccel);
  return (2 * maxSpeed) / maxAccel + (dist - 2 * dAccel) / maxSpeed;
}

// ── Pass fan ─────────────────────────────────────────────────────
//
// 2–3 candidate passes, each scored by corridor clearance (defenders'
// velocity-aware interception), receiver control, and xT gain. Replaces the
// single static lane with a small calibrated set.
export function passFan(ball, candidates, opponents, dir) {
  if (!ball || !candidates?.length) return [];
  const scored = [];
  for (const c of candidates) {
    if (!c || !Number.isFinite(c.x) || !Number.isFinite(c.y)) continue;
    const dist = distance(ball, c);
    if (dist < ANALYSIS.laneMinBallDistance || dist > ANALYSIS.laneMaxBallDistance) continue;
    const flightTime = Math.max(0.35, dist / 20);
    const reception = predictPosition(c, flightTime);
    const { clearance, beats } = corridorClearance(ball, reception, opponents, flightTime);
    if (!beats) continue;
    const control = clamp01(1.2 - dist / 45);
    const value = xtGain(ball, reception, dir);
    const p = clamp01(
      clamp01(clearance / 6) * 0.55 + control * 0.25 + clamp01(0.5 + value * 4) * 0.2,
    );
    scored.push({ from: { ...ball }, to: reception, player: c, p, clearance, value });
  }
  scored.sort((a, b) => b.p - a.p);
  return scored.slice(0, ANALYSIS.fanMaxOptions);
}

/**
 * Corridor clearance: sample the ball path every ~0.5 m; at each sample,
 * check whether any defender reaches it before the ball (defender arrival
 * vs ball arrival + margin). Returns the minimum clearance and whether the
 * ball beats every defender.
 */
export function corridorClearance(from, to, opponents, flightTime) {
  const dist = distance(from, to);
  const steps = Math.max(2, Math.round(dist / ANALYSIS.fanSampleStep));
  let clearance = Infinity;
  for (let s = 0; s <= steps; s++) {
    const u = s / steps;
    const px = from.x + (to.x - from.x) * u;
    const py = from.y + (to.y - from.y) * u;
    const ballT = flightTime * u;
    for (const o of opponents ?? []) {
      if (!o || !Number.isFinite(o.x) || !Number.isFinite(o.y)) continue;
      const d = Math.hypot(o.x - px, o.y - py);
      clearance = Math.min(clearance, d);
      // Defender arrival from current velocity (stop-to-stop, capped accel).
      const oSpeed = Math.hypot(o.vx || 0, o.vy || 0);
      const defT = arrivalTime(Math.max(0, d - 0.5), Math.max(4, oSpeed + 2), ANALYSIS.fanDefenderAccel);
      if (defT < ballT + ANALYSIS.fanInterceptMargin && d < 6) return { clearance, beats: false };
    }
  }
  return { clearance: Number.isFinite(clearance) ? clearance : 99, beats: true };
}

// ── Space control ────────────────────────────────────────────────
//
// Local grid around the threat: cells the attacker reaches a beat before the
// nearest defender. Naturally concave, shrinks under pressure, and means
// something to an audience ("space he can actually use").
export function spaceControl(threat, opponents) {
  if (!threat?.player) return { cells: [], area: 0 };
  const p = threat.player;
  const half = ANALYSIS.spaceGridHalf;
  const step = ANALYSIS.spaceGridStep;
  const cells = [];
  for (let gx = -half; gx <= half + 1e-6; gx += step) {
    for (let gy = -half; gy <= half + 1e-6; gy += step) {
      const cell = { x: p.x + gx, y: p.y + gy };
      if (Math.abs(cell.x) > 52.5 || Math.abs(cell.y) > 34) continue;
      const atkT = arrivalTime(Math.hypot(gx, gy), p.speed || 6, 6);
      let defT = Infinity;
      for (const o of opponents ?? []) {
        if (!o || !Number.isFinite(o.x)) continue;
        defT = Math.min(defT, arrivalTime(distance(o, cell), Math.hypot(o.vx || 0, o.vy || 0) + 3, 6));
      }
      if (atkT + ANALYSIS.spaceWinMargin < defT) cells.push(cell);
    }
  }
  return { cells, area: cells.length * step * step };
}

// ── Defensive-line analysis ──────────────────────────────────────
export function backLineStats(teamPlayers, dir) {
  const outfield = (teamPlayers ?? []).filter((p) => p.role !== ROLE.GK);
  if (outfield.length < 4) return null;
  const prog = (p) => p.x * dir;
  const sorted = outfield.slice().sort((a, b) => prog(a) - prog(b));
  const deepest = sorted.slice(0, 4);
  const xs = deepest.map((p) => p.x);
  const ys = deepest.map((p) => p.y).sort((a, b) => a - b);
  let maxGap = 0;
  for (let i = 1; i < ys.length; i++) maxGap = Math.max(maxGap, ys[i] - ys[i - 1]);
  const defMean = xs.reduce((s, v) => s + v, 0) / xs.length;
  const mids = outfield.filter((p) => p.role === ROLE.MID);
  const fwds = outfield.filter((p) => p.role === ROLE.FWD);
  const mean = (ps) => (ps.length ? ps.reduce((s, p) => s + prog(p), 0) / ps.length : NaN);
  return {
    height: defMean,
    width: ys[ys.length - 1] - ys[0],
    maxGap,
    defMidGap: mean(mids) - defMean,
    midFwdGap: mean(fwds) - mean(mids),
  };
}

// ── Threat grammar ───────────────────────────────────────────────
// Classify HOW the threat is dangerous: feet corridor, through channel,
// cross/cutback zone, or shot cone. One clear primitive at a time.
export function threatGrammar(threat, ball, opponents, dir) {
  if (!threat?.player) return { kind: 'none' };
  const p = threat.player;
  const goal = { x: dir * 52.5, y: 0 };
  const ballDist = distance(p, ball);
  const goalDist = distance(p, goal);
  const wide = Math.abs(ball.x * dir) > 20 && Math.abs(ball.y) > 18;
  if (goalDist < 26 && ballDist < 30) {
    return { kind: 'shot', xg: shotXg(p.x, p.y, dir), goal };
  }
  if (wide && p.x * dir > 5) {
    return { kind: 'cross', zone: crossZone(p, dir) };
  }
  // Through channel: space behind the opponent line on the threat's side.
  const ahead = opponents.filter((o) => o.x * dir > p.x * dir);
  if (ahead.length > 0) {
    return { kind: 'through', target: predictPosition(p, 1.2) };
  }
  return { kind: 'feet' };
}

/** Near/far/spot cross target polygon (shaded zone, not a line). */
export function crossZone(threat, dir) {
  const gx = dir * 52.5;
  const side = Math.sign(threat.y || 1);
  return [
    { x: gx - dir * 6, y: side * 6 },
    { x: gx - dir * 6, y: -side * 6 },
    { x: gx - dir * 12, y: 0 },
  ];
}

// ── Team shape: three unit segments + centroid triangle ─────────
/**
 * Split outfield players into defensive, midfield and attacking units.
 * Roles are preferred; when they are missing or leave a unit empty, the
 * outfielders are grouped by attacking depth (deepest 4 → DEF, then the
 * next band → MID, the rest → FWD) so the renderer can always connect three
 * lines. The GK is excluded; each unit is ordered top-to-bottom in world y.
 */
export function splitUnits(teamPlayers, dir = 1) {
  const players = (teamPlayers ?? []).filter((p) => p.role !== ROLE.GK);
  const units = { def: [], mid: [], fwd: [] };
  for (const p of players) {
    if (p.role === ROLE.DEF) units.def.push(p);
    else if (p.role === ROLE.MID) units.mid.push(p);
    else if (p.role === ROLE.FWD) units.fwd.push(p);
  }
  const complete = units.def.length > 0 && units.mid.length > 0 && units.fwd.length > 0;
  if (!complete && players.length >= 6) {
    const sorted = players.slice().sort((a, b) => a.x * dir - b.x * dir);
    const n = sorted.length;
    const defCount = Math.min(4, Math.max(1, Math.round(n * 0.36)));
    const fwdCount = Math.min(3, Math.max(1, Math.round(n * 0.3)));
    const midCount = Math.max(1, n - defCount - fwdCount);
    units.def = sorted.slice(0, defCount);
    units.mid = sorted.slice(defCount, defCount + midCount);
    units.fwd = sorted.slice(defCount + midCount);
  }
  for (const key of ['def', 'mid', 'fwd']) units[key].sort((a, b) => b.y - a.y);
  return units;
}

/** Apply a role array to the current (transient) player snapshots. */
export function applyRoles(players, roles) {
  players.forEach((p, i) => {
    p.role = roles[i] ?? ROLE.UNKNOWN;
  });
}

/** Mean position of a unit, or null when the unit is empty. */
export function unitCentroid(unit) {
  if (!unit || unit.length === 0) return null;
  let x = 0;
  let y = 0;
  for (const p of unit) {
    x += p.x;
    y += p.y;
  }
  return { x: x / unit.length, y: y / unit.length };
}

/** Centroids of the three units, or null unless all three exist. */
export function shapeTriangle(units) {
  if (!units) return null;
  const def = unitCentroid(units.def);
  const mid = unitCentroid(units.mid);
  const fwd = unitCentroid(units.fwd);
  if (!def || !mid || !fwd) return null;
  return { def, mid, fwd };
}
