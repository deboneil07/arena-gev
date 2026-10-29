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

/** Pitch extents in engine metres (origin at the centre spot). */
export const PITCH = Object.freeze({ halfLength: 52.5, halfWidth: 34 });

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

  // Attacking lanes: four equal bands across the pitch width, drawn from
  // halfway to the goal being attacked. The projection horizon is short
  // enough to read as "where he is going next", not a full sprint model.
  attackLaneCount: 4,
  attackLaneHorizon: 1.8, // seconds of velocity + acceleration projection
  attackLaneCarrierRadius: 3.5, // metres from ball to identify the carrier
  attackLaneOptionLimit: 12, // score every viable receiver before bucketing

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

/**
 * Index of the horizontal channel containing `y`, 0 = bottom touchline and
 * `count - 1` = top touchline. Four 17 m bands by default.
 */
export function laneIndex(y, count = ANALYSIS.attackLaneCount) {
  const n = Math.max(1, Math.floor(count) || 1);
  const width = (2 * PITCH.halfWidth) / n;
  const value = Number.isFinite(y) ? y : 0;
  const i = Math.floor((value + PITCH.halfWidth) / width);
  return Math.max(0, Math.min(n - 1, i));
}

/** World-space extents of one horizontal channel. */
export function laneBand(index, count = ANALYSIS.attackLaneCount) {
  const n = Math.max(1, Math.floor(count) || 1);
  const i = Math.max(0, Math.min(n - 1, Math.floor(index) || 0));
  const width = (2 * PITCH.halfWidth) / n;
  const y0 = -PITCH.halfWidth + i * width;
  return { index: i, y0, y1: y0 + width, centerY: y0 + width / 2, width };
}

/** Centre line of a horizontal channel. */
export function laneCenterY(index, count = ANALYSIS.attackLaneCount) {
  return laneBand(index, count).centerY;
}

const SHAPE_LABELS = Object.freeze({
  '4-4-2': '4-4-2',
  '4-3-3': '4-3-3',
  '4-5-1': '4-2-3-1',
  '3-5-2': '3-5-2',
  '5-3-2': '5-3-2',
  '4-1-1': '4-1-4-1',
});

export function clamp01(value) {
  if (typeof value !== 'number' || Number.isNaN(value)) return 0;
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
  if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(dir)) return 0;
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
  if (!Number.isFinite(x) || !Number.isFinite(y) || !Number.isFinite(dir)) return 0.02;
  const px = x * dir;
  const d = Math.hypot(52.5 - px, y);
  if (d > 35) return 0.02;
  const angle = Math.abs(Math.atan2(Math.abs(y), Math.max(1, 52.5 - px)));
  const base = clamp01(1 - d / 32);
  return Number(clamp01(0.08 + 0.55 * base * (0.5 + 0.5 * Math.cos(angle))).toFixed(3));
}

/** Predict a player's position `dt` seconds ahead from velocity. */
export function predictPosition(p, dt) {
  if (!p || !Number.isFinite(p.x) || !Number.isFinite(p.y)) return { x: 0, y: 0 };
  const step = Number.isFinite(dt) ? dt : 0;
  return {
    x: p.x + (p.vx || 0) * step,
    y: p.y + (p.vy || 0) * step,
  };
}

/** Stop-to-stop time for `dist` metres at caps (mirrors the engine). */
export function arrivalTime(dist, maxSpeed = 8.0, maxAccel = 6.0) {
  if (!Number.isFinite(dist) || dist <= 0) return 0;
  if (!Number.isFinite(maxSpeed) || maxSpeed <= 0) maxSpeed = 8.0;
  if (!Number.isFinite(maxAccel) || maxAccel <= 0) maxAccel = 6.0;
  const dAccel = (maxSpeed * maxSpeed) / (2 * maxAccel);
  if (dist <= 2 * dAccel) return 2 * Math.sqrt(dist / maxAccel);
  return (2 * maxSpeed) / maxAccel + (dist - 2 * dAccel) / maxSpeed;
}

// ── Pass fan ─────────────────────────────────────────────────────
//
// 2–3 candidate passes, each scored by corridor clearance (defenders'
// velocity-aware interception), receiver control, and xT gain. Replaces the
// single static lane with a small calibrated set.
export function passFan(ball, candidates, opponents, dir, limit = ANALYSIS.fanMaxOptions) {
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
    // Nearest opponent to the reception point: pressure lowers completion.
    let receiverPressure = 12;
    for (const o of opponents ?? []) {
      if (!o || !Number.isFinite(o.x) || !Number.isFinite(o.y)) continue;
      receiverPressure = Math.min(receiverPressure, Math.hypot(o.x - reception.x, o.y - reception.y));
    }
    const pressureScore = clamp01(receiverPressure / 6);
    // Logistic completion probability: clearance carries the most weight,
    // then control, receiver pressure and the value of the space gained.
    // The distance term keeps a 50 m switch below a short, clean ball.
    const logit =
      -1.2 +
      1.9 * clamp01(clearance / 6) +
      1.3 * control +
      0.8 * pressureScore +
      0.6 * clamp01(0.5 + value * 4) -
      0.03 * dist;
    const p = clamp01(1 / (1 + Math.exp(-logit)));
    scored.push({ from: { ...ball }, to: reception, player: c, p, clearance, value, pressure: receiverPressure });
  }
  scored.sort((a, b) => b.p - a.p);
  return scored.slice(0, Math.max(0, limit));
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

/**
 * Live state of the four attacking channels.
 *
 * Active only for the team in possession, once the ball is on the attacking
 * half. Each lane combines:
 * - `momentum`: the carrier's velocity + acceleration projected ~1.8 s ahead,
 *   so the lane he is actually heading into burns brightest;
 * - `option`: the best viable pass into that lane (`passFan` completion
 *   probability), so the lanes also answer "where can he play it?";
 * - a small baseline on the carrier's current lane.
 *
 * Pure: the renderer only supplies snapshots and draws the result.
 */
export function attackLaneState(players, ball, team, dir, opponents = null, carrier = null) {
  const empty = {
    active: false,
    team,
    dir: dir === -1 ? -1 : 1,
    carrier: null,
    carrierLane: null,
    projectedLane: null,
    projected: null,
    lanes: [],
    bestPass: null,
    progress: -Infinity,
  };
  if (!players?.length || !ball) return empty;
  const attackDir = dir === -1 ? -1 : 1;
  const teamPlayers = players.filter((p) => p.team === team);
  if (teamPlayers.length === 0) return empty;

  let man = carrier;
  if (!man) {
    let bestD = Infinity;
    for (const p of teamPlayers) {
      const d = Math.hypot(p.x - ball.x, p.y - ball.y);
      if (d < bestD) { bestD = d; man = p; }
    }
    if (bestD > ANALYSIS.attackLaneCarrierRadius) return empty;
  }

  const opps = opponents ?? players.filter((p) => p.team !== team);
  const options = passFan(
    ball,
    teamPlayers.filter((p) => p !== man),
    opps,
    attackDir,
    ANALYSIS.attackLaneOptionLimit,
  );
  const bestPass = options[0] ?? null;
  const progress = ball.x * attackDir;
  const carrierLane = laneIndex(man.y);
  if (progress < 0) {
    return { ...empty, carrier: man, carrierLane, bestPass, progress };
  }

  const t = ANALYSIS.attackLaneHorizon;
  const vx = Number.isFinite(man.vx) ? man.vx : 0;
  const vy = Number.isFinite(man.vy) ? man.vy : 0;
  const ax = Number.isFinite(man.ax) ? man.ax : 0;
  const ay = Number.isFinite(man.ay) ? man.ay : 0;
  const projected = {
    x: man.x + vx * t + 0.5 * ax * t * t,
    y: man.y + vy * t + 0.5 * ay * t * t,
  };
  projected.y = Math.max(-PITCH.halfWidth, Math.min(PITCH.halfWidth, projected.y));
  const projectedLane = laneIndex(projected.y);
  const along = clamp01((vx * attackDir) / 6.5);
  const cut = clamp01(Math.abs(vy) / 5.5);

  const lanes = [];
  for (let i = 0; i < ANALYSIS.attackLaneCount; i++) {
    const band = laneBand(i);
    let option = 0;
    let target = null;
    for (const o of options) {
      if (laneIndex(o.to.y) !== i) continue;
      const p = clamp01(o.p);
      if (p > option) {
        option = p;
        target = {
          x: o.to.x,
          y: o.to.y,
          shirt: o.player?.shirt ?? null,
          team: o.player?.team ?? null,
          p,
        };
      }
    }
    // Forward momentum dominates; a lateral cut still lights the lane it is
    // heading into, but a pass backwards does not pretend to be an attack.
    const momentum = i === projectedLane
      ? clamp01(0.1 + 0.75 * along + 0.15 * cut)
      : 0;
    const carrierHere = i === carrierLane;
    const score = clamp01(0.1 + 0.72 * momentum + 0.62 * option + (carrierHere ? 0.08 : 0));
    lanes.push({
      index: i,
      y0: band.y0,
      y1: band.y1,
      centerY: band.centerY,
      width: band.width,
      x0: 0,
      x1: attackDir * PITCH.halfLength,
      score,
      momentum,
      option,
      projected: i === projectedLane,
      carrierLane: carrierHere,
      target,
    });
  }

  return {
    active: true,
    team,
    dir: attackDir,
    carrier: man,
    carrierLane,
    projectedLane,
    projected,
    lanes,
    bestPass,
    progress,
  };
}

/**
 * Perpendicular bow (metres) for a run channel, curving away from the closest
 * defender on the straight line. Zero when the lane is already clean.
 */
function runCurveOffset(from, to, opponents) {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const len = Math.hypot(dx, dy);
  if (len < 1e-6) return 0;
  const nx = -dy / len;
  const ny = dx / len;
  const mx = (from.x + to.x) / 2;
  const my = (from.y + to.y) / 2;
  let nearest = null;
  let nearestD = Infinity;
  for (const o of opponents ?? []) {
    if (!o || !Number.isFinite(o.x) || !Number.isFinite(o.y)) continue;
    const d = pointSegmentDistance(o, from, to);
    if (d < nearestD) {
      nearestD = d;
      nearest = o;
    }
  }
  if (!nearest || nearestD > 9) return 0;
  // Which side of the line is the defender on?
  const side = (nearest.x - mx) * nx + (nearest.y - my) * ny;
  const sign = side >= 0 ? -1 : 1;
  // Keep the bow readable: never more than ~22% of the run length.
  const amount = Math.min(len * 0.22, Math.max(1.0, (9 - nearestD) * 0.55));
  return sign * amount;
}

/**
 * Attack channels: probable runs forwards SHOULD make during a counter.
 * Returns [{ from:{x,y}, to:{x,y}, player, score, curve }].
 */
export function attackChannels(players, ball, team, dir, opponents) {
  if (!players?.length || !ball) return [];
  const teamPlayers = players.filter((p) => p.team === team);
  if (teamPlayers.length === 0) return [];
  const progress = ball.x * dir;
  if (progress < 15) return [];
  const opps = opponents ?? players.filter((p) => p.team !== team);
  const goal = { x: dir * 52.5, y: 0 };
  const forwards = teamPlayers.filter(
    (p) => (p.role === ROLE.FWD || p.role === ROLE.MID) && distance(p, ball) > 2.5,
  );
  const advanced = forwards.filter((p) => p.x * dir > progress - 5);
  const targets = advanced.length
    ? advanced
    : forwards.slice().sort((a, b) => b.x * dir - a.x * dir).slice(0, 3);
  const out = [];

  for (const p of targets) {
    if (!Number.isFinite(p.x) || !Number.isFinite(p.y)) continue;
    const baseValue = xtAt(p.x, p.y, dir);
    const goalAngle = Math.atan2(goal.y - p.y, goal.x - p.x);
    const speed = Number.isFinite(p.speed) && p.speed > 1 ? p.speed : 7;
    let best = null;

    // Sample short, readable run endpoints: 6/9/12 m ahead, fanned 20°
    // around the goal vector. Score by xT gain and the time the runner gains
    // on the nearest defender.
    for (const radius of [6, 9, 12]) {
      for (const da of [-0.35, -0.175, 0, 0.175, 0.35]) {
        const ang = goalAngle + da;
        let ex = p.x + Math.cos(ang) * radius;
        let ey = p.y + Math.sin(ang) * radius;
        ex = Math.max(-52.5, Math.min(52.5, ex));
        ey = Math.max(-34, Math.min(34, ey));
        const gain = xtAt(ex, ey, dir) - baseValue;
        if (gain <= 0) continue;

        const runT = arrivalTime(Math.hypot(ex - p.x, ey - p.y), speed, 5.5);
        let defT = Infinity;
        for (const o of opps) {
          if (!o || !Number.isFinite(o.x) || !Number.isFinite(o.y)) continue;
          const oSpeed = Math.hypot(o.vx || 0, o.vy || 0);
          defT = Math.min(defT, arrivalTime(distance(o, { x: ex, y: ey }), Math.max(4, oSpeed + 2), 6));
        }
        const margin = Number.isFinite(defT) ? defT - runT : 3;
        const laneClear = passLaneClear(ball, { x: ex, y: ey }, opps, 2.5) ? 1 : 0;
        const score = clamp01(
          0.55 * clamp01(gain * 4) +
          0.35 * clamp01(margin / 2) +
          0.10 * laneClear,
        );
        if (!best || score > best.score) {
          best = { x: ex, y: ey, score, gain, margin };
        }
      }
    }

    if (!best) continue;
    // Where the run lands, expressed as an attacking channel. Only runs that
    // finish on the attacking half get a lane landing; a drop back to build
    // is not a threat.
    const landingLane = laneIndex(best.y);
    const landingBand = laneBand(landingLane);
    const landing = best.x * dir >= 0
      ? {
          lane: landingLane,
          y0: landingBand.y0,
          y1: landingBand.y1,
          centerY: landingBand.centerY,
          x0: 0,
          x1: dir * PITCH.halfLength,
          x: best.x,
          y: best.y,
        }
      : null;
    out.push({
      from: { x: p.x, y: p.y },
      to: { x: best.x, y: best.y },
      player: p,
      score: best.score,
      gain: best.gain,
      margin: best.margin,
      curve: runCurveOffset({ x: p.x, y: p.y }, { x: best.x, y: best.y }, opps),
      landing,
    });
  }

  out.sort((a, b) => b.score - a.score);
  return out.slice(0, 3);
}

/** Stable per-player hash (team + shirt), no engine id needed. */
function playerHash(player) {
  const key = `${player?.team ?? 0}:${player?.shirt ?? 0}`;
  let h = 2166136261;
  for (let i = 0; i < key.length; i++) {
    h ^= key.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  // Final avalanche so the low and high bit fields are independent.
  h ^= h >>> 16;
  h = Math.imul(h, 2246822507);
  h ^= h >>> 13;
  h = Math.imul(h, 3266489909);
  h ^= h >>> 16;
  return h >>> 0;
}

/**
 * Role-based vision model.
 *
 * `near` is the player's immediate awareness (the familiar cone and blind
 * circle); `reach` is the long view the raycast layer traces across the pitch.
 * Keepers see almost the whole field, defenders read a long way, midfielders
 * scan a little further than forwards, and forwards work in the tightest
 * window. The per-player modifier is stable (team + shirt) and small, so two
 * players in the same role still differ slightly without ever reading as
 * random.
 */
export const VISION = Object.freeze({
  near: Object.freeze({
    [ROLE.GK]: Object.freeze({ radius: 26, halfAngle: 80 }),
    [ROLE.DEF]: Object.freeze({ radius: 24, halfAngle: 70 }),
    [ROLE.MID]: Object.freeze({ radius: 22, halfAngle: 62 }),
    [ROLE.FWD]: Object.freeze({ radius: 20, halfAngle: 54 }),
    [ROLE.UNKNOWN]: Object.freeze({ radius: 22, halfAngle: 62 }),
  }),
  reach: Object.freeze({
    [ROLE.GK]: Object.freeze({ distance: 58, halfAngle: 160 }),
    [ROLE.DEF]: Object.freeze({ distance: 44, halfAngle: 110 }),
    [ROLE.MID]: Object.freeze({ distance: 36, halfAngle: 100 }),
    [ROLE.FWD]: Object.freeze({ distance: 30, halfAngle: 90 }),
    [ROLE.UNKNOWN]: Object.freeze({ distance: 34, halfAngle: 100 }),
  }),
  // Angular falloff turns the hard sector into light: brightness peaks at the
  // facing direction and decays toward the edge. Keepers keep a high edge
  // value (near all-round peripheral awareness); each line forward steepens
  // the beam until a striker is a tight torch.
  falloff: Object.freeze({
    [ROLE.GK]: Object.freeze({ power: 1.1, edge: 0.62 }),
    [ROLE.DEF]: Object.freeze({ power: 1.5, edge: 0.42 }),
    [ROLE.MID]: Object.freeze({ power: 2.0, edge: 0.28 }),
    [ROLE.FWD]: Object.freeze({ power: 2.8, edge: 0.12 }),
    [ROLE.UNKNOWN]: Object.freeze({ power: 2.0, edge: 0.28 }),
  }),
  playerVariation: 0.08, // ±8 % stable per-player modifier
  blockerRadius: 0.9, // metres; a body is a disc on the sightline
  ownBlockerRadius: 0.75, // the keeper's own men occlude a little softer
  rays: 64, // uniform samples between the exact tangent rays
  penumbra: 0.45, // metres of soft edge around each shadow
});

const DEG = Math.PI / 180;

/**
 * Relative brightness at `t` of the way from the facing direction to the
 * cone edge. 1 at the focus, `edge` at the rim; `power` controls how fast it
 * falls away in between.
 */
export function visionFalloff(t, power = 2.0, edge = 0.28) {
  const x = Math.min(1, Math.abs(Number.isFinite(t) ? t : 0));
  const p = Number.isFinite(power) && power > 0 ? power : 2.0;
  const e = clamp01(edge);
  return e + (1 - e) * Math.pow(1 - x, p);
}

/** Role-first vision profile: near cone plus the long raycast reach. */
export function visionProfile(player) {
  const role = player && VISION.near[player.role] ? player.role : ROLE.UNKNOWN;
  const near = VISION.near[role];
  const reach = VISION.reach[role];
  const falloff = VISION.falloff[role];
  const h = playerHash(player ?? {});
  const radiusScale = 1 + (((h & 0xff) / 0xff) * 2 - 1) * VISION.playerVariation;
  const angleScale =
    1 + ((((h >>> 8) & 0xff) / 0xff) * 2 - 1) * VISION.playerVariation * 0.6;
  return {
    role,
    radius: near.radius * radiusScale,
    halfAngle: near.halfAngle * DEG * angleScale,
    reach: reach.distance * radiusScale,
    longHalfAngle: reach.halfAngle * DEG * angleScale,
    falloffPower: falloff.power,
    falloffEdge: falloff.edge,
  };
}

/** Where a player looks: best pass > heading > velocity > attacking direction. */
function visionFacing(player, dir, focus = null) {
  // The analytical focus (the best passing option) wins over the raw engine
  // heading: players should look where the next pass is going, not at the
  // ball at their feet.
  if (
    focus &&
    Number.isFinite(focus.x) &&
    Number.isFinite(focus.y) &&
    Math.hypot(focus.x - player.x, focus.y - player.y) > 1.0
  ) {
    return Math.atan2(focus.y - player.y, focus.x - player.x);
  }
  if (Number.isFinite(player.heading) && Math.abs(player.heading) > 1e-6) {
    return player.heading;
  }
  if (Math.hypot(player.vx || 0, player.vy || 0) > 0.2) {
    return Math.atan2(player.vy || 0, player.vx || 0);
  }
  return dir > 0 ? 0 : Math.PI;
}

/**
 * Sightline blockers. Opponents always block; a keeper is additionally
 * screened by his own defenders (the bodies in front of him), which is why
 * the GK's own-team shadow reads softer than an opponent's.
 */
export function visionBlockers(player, teammates, opponents) {
  const blockers = [];
  for (const o of opponents ?? []) {
    if (!o || !Number.isFinite(o.x) || !Number.isFinite(o.y)) continue;
    blockers.push({ x: o.x, y: o.y, radius: VISION.blockerRadius, own: false });
  }
  if (player && player.role === ROLE.GK) {
    for (const t of teammates ?? []) {
      if (!t || t === player || !Number.isFinite(t.x) || !Number.isFinite(t.y)) continue;
      blockers.push({ x: t.x, y: t.y, radius: VISION.ownBlockerRadius, own: true });
    }
  }
  return blockers;
}

/**
 * Vision cone geometry for a player: facing direction, cone sector, and
 * classification of teammates into visible (inside cone) vs blind (outside).
 */
export function visionCone(player, teammates, opponents, dir, focus = null) {
  const profile = visionProfile(player ?? {});
  if (!player || !Number.isFinite(player.x) || !Number.isFinite(player.y)) {
    return {
      apex: null,
      facing: 0,
      halfAngle: profile.halfAngle,
      radius: profile.radius,
      conePoints: [],
      blindConePoints: [],
      coneFalloff: [],
      visible: [],
      blind: [],
      coverage: 0,
    };
  }
  const facing = visionFacing(player, dir, focus);
  const { halfAngle, radius } = profile;
  // 24 sectors keep the falloff smooth enough to read as light; the old 12
  // left visible fan steps inside a wide keeper cone.
  const segs = 24;
  const conePoints = [{ x: player.x, y: player.y }];
  const coneFalloff = [];
  for (let i = 0; i <= segs; i++) {
    const a = facing - halfAngle + (2 * halfAngle * i) / segs;
    conePoints.push({ x: player.x + Math.cos(a) * radius, y: player.y + Math.sin(a) * radius });
    coneFalloff.push(visionFalloff((a - facing) / halfAngle, profile.falloffPower, profile.falloffEdge));
  }
  conePoints.push({ x: player.x, y: player.y });
  // Blind sector = rest of the circle behind/around the cone.
  const blindConePoints = [{ x: player.x, y: player.y }];
  const startA = facing + halfAngle;
  const blindSweep = 2 * Math.PI - 2 * halfAngle;
  for (let i = 0; i <= 16; i++) {
    const a = startA + (blindSweep * i) / 16;
    blindConePoints.push({ x: player.x + Math.cos(a) * radius, y: player.y + Math.sin(a) * radius });
  }
  blindConePoints.push({ x: player.x, y: player.y });
  const inCone = (q) => {
    if (!q || q === player) return false;
    const dx = q.x - player.x;
    const dy = q.y - player.y;
    const d = Math.hypot(dx, dy);
    if (d < 1e-6 || d > radius) return false;
    let ang = Math.atan2(dy, dx);
    let diff = ang - facing;
    while (diff > Math.PI) diff -= 2 * Math.PI;
    while (diff < -Math.PI) diff += 2 * Math.PI;
    return Math.abs(diff) <= halfAngle;
  };
  // A teammate behind a body on the sightline is not actually visible.
  const blockers = visionBlockers(player, teammates, opponents);
  const occluded = (q) => {
    for (const b of blockers) {
      if (pointSegmentDistance(b, player, q) < b.radius + 0.3) return true;
    }
    return false;
  };
  const visible = [];
  const blind = [];
  for (const t of teammates ?? []) {
    if (t === player) continue;
    // Only nearby teammates belong in the vision picture; a 40 m teammate
    // behind play is noise, not a passing option.
    if (Math.hypot(t.x - player.x, t.y - player.y) > radius) continue;
    (inCone(t) && !occluded(t) ? visible : blind).push(t);
  }
  return {
    apex: { x: player.x, y: player.y },
    facing,
    halfAngle,
    radius,
    conePoints,
    coneFalloff,
    blindConePoints,
    visible,
    blind,
    coverage: visible.length + blind.length > 0
      ? visible.length / (visible.length + blind.length)
      : 0,
  };
}

/** Four-point wedge behind a blocker, from its silhouette out to the reach. */
function shadowWedge(apex, blocker, tangent, nearEdge, reach) {
  const a1 = blocker.angle - tangent;
  const a2 = blocker.angle + tangent;
  const point = (a, t) => ({ x: apex.x + Math.cos(a) * t, y: apex.y + Math.sin(a) * t });
  return [point(a1, nearEdge), point(a1, reach), point(a2, reach), point(a2, nearEdge)];
}

/**
 * Raycast long view: the lit polygon around a player, clipped by every
 * blocking body, plus the shadow wedge each blocker casts. Exact tangent
 * angles are added to the uniform arc samples, so shadow edges land on the
 * blocker silhouette rather than between samples. Pure geometry — the
 * renderer turns `polygon`, `sector` and `shadows` into the light/shadow
 * layer.
 */
export function raycastVision(player, blockers, dir, focus = null, profile = null) {
  const p = profile ?? visionProfile(player ?? {});
  if (!player || !Number.isFinite(player.x) || !Number.isFinite(player.y)) {
    return {
      apex: null,
      facing: 0,
      reach: p.reach,
      halfAngle: p.longHalfAngle,
      polygon: [],
      sector: [],
      shadows: [],
      falloff: [],
    };
  }
  const apex = { x: player.x, y: player.y };
  const facing = visionFacing(player, dir, focus);
  const reach = p.reach;
  const half = p.longHalfAngle;
  const start = facing - half;
  const end = facing + half;

  // Only bodies inside the reach and clear of the player can matter.
  const active = [];
  for (const b of blockers ?? []) {
    if (!b || !Number.isFinite(b.x) || !Number.isFinite(b.y)) continue;
    const dx = b.x - apex.x;
    const dy = b.y - apex.y;
    const d = Math.hypot(dx, dy);
    const r = Math.max(0.2, b.radius ?? VISION.blockerRadius);
    if (d <= r + 1e-3 || d - r > reach) continue;
    active.push({ ...b, dx, dy, d, r, angle: Math.atan2(dy, dx) });
  }

  const angles = [];
  for (let i = 0; i <= VISION.rays; i++) {
    angles.push(start + ((end - start) * i) / VISION.rays);
  }
  for (const b of active) {
    const tangent = Math.asin(Math.min(1, b.r / b.d));
    for (const a of [b.angle - tangent, b.angle + tangent]) {
      if (a >= start - 1e-6 && a <= end + 1e-6) angles.push(a);
    }
  }
  angles.sort((a, b) => a - b);

  const hitDistance = (angle) => {
    const ux = Math.cos(angle);
    const uy = Math.sin(angle);
    let nearest = reach;
    for (const b of active) {
      const proj = b.dx * ux + b.dy * uy;
      if (proj <= 0) continue;
      const perp2 = b.dx * b.dx + b.dy * b.dy - proj * proj;
      const r2 = b.r * b.r;
      if (perp2 >= r2) continue;
      const t = proj - Math.sqrt(r2 - perp2);
      if (t > 0 && t < nearest) nearest = t;
    }
    return nearest;
  };

  const polygon = [{ ...apex }];
  const falloff = [1];
  for (const a of angles) {
    const t = hitDistance(a);
    polygon.push({ x: apex.x + Math.cos(a) * t, y: apex.y + Math.sin(a) * t });
    falloff.push(visionFalloff((a - facing) / half, p.falloffPower, p.falloffEdge));
  }

  const sector = [{ ...apex }];
  for (let i = 0; i <= VISION.rays; i++) {
    const a = start + ((end - start) * i) / VISION.rays;
    sector.push({ x: apex.x + Math.cos(a) * reach, y: apex.y + Math.sin(a) * reach });
  }

  const shadows = [];
  for (const b of active) {
    const tangent = Math.asin(Math.min(1, b.r / b.d));
    const nearEdge = Math.sqrt(Math.max(0, b.d * b.d - b.r * b.r));
    const softTangent = Math.asin(Math.min(1, (b.r + VISION.penumbra) / b.d));
    const softNear = Math.sqrt(
      Math.max(0, b.d * b.d - (b.r + VISION.penumbra) * (b.r + VISION.penumbra)),
    );
    shadows.push({
      points: shadowWedge(apex, b, tangent, nearEdge, reach),
      penumbra: shadowWedge(apex, b, softTangent, softNear, reach),
      own: Boolean(b.own),
    });
  }

  return { apex, facing, reach, halfAngle: half, polygon, sector, shadows, falloff };
}

/**
 * Offside-line usefulness. The line is a reference, not a fixture: it is
 * dropped when it would describe nothing an analyst can act on.
 *
 * - `deep-back-line`: the second-last defender is camped within 15 m of the
 *   goal he defends, so the line sits on the penalty box and there is no
 *   space behind it to attack.
 * - `line-at-ball`: the engine clamps the line to the ball when the ball is
 *   beyond the back line; the ball marker already is the line.
 * - `no-attacker-near`: no attacker within 18 m of the line, so it currently
 *   constrains nobody.
 */
export const OFFSIDE = Object.freeze({
  deepGoalDistance: 15.0, // metres from the defended goal
  ballCoincidence: 2.5, // metres; line this close to the ball is the ball
  attackerBand: 18.0, // metres; nearest attacker beyond this = no pressure
});

export function offsideLineVisible({ offsideX, players, possessionTeam, ball } = {}) {
  if (possessionTeam !== 0 && possessionTeam !== 1) {
    return { show: false, reason: 'no-possession' };
  }
  if (!Number.isFinite(offsideX)) return { show: false, reason: 'no-line' };
  const dir = possessionTeam === 0 ? 1 : -1;
  // Distance from the line to the goal the possession team attacks.
  const goalDistance = 52.5 - offsideX * dir;
  if (goalDistance < OFFSIDE.deepGoalDistance) {
    return { show: false, reason: 'deep-back-line' };
  }
  if (ball && Number.isFinite(ball.x) && Math.abs(offsideX - ball.x) < OFFSIDE.ballCoincidence) {
    return { show: false, reason: 'line-at-ball' };
  }
  const attackers = (players ?? []).filter(
    (p) => p && p.team === possessionTeam && p.role !== ROLE.GK,
  );
  if (attackers.length > 0) {
    let nearest = Infinity;
    for (const p of attackers) {
      const d = Math.abs(p.x - offsideX);
      if (d < nearest) nearest = d;
    }
    if (nearest > OFFSIDE.attackerBand) return { show: false, reason: 'no-attacker-near' };
  }
  return { show: true, reason: 'open-play' };
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
