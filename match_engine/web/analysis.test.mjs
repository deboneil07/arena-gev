import test from 'node:test';
import assert from 'node:assert/strict';

import {
  ROLE,
  ANALYSIS,
  applyRoles,
  attackingCandidates,
  attackChannels,
  backLineStats,
  buildDangerZone,
  clamp01,
  convexHull,
  corridorClearance,
  crossZone,
  detectMidfieldBank,
  detectMidfieldBanks,
  evaluateThreats,
  formationLabel,
  passFan,
  passLaneClear,
  passingLaneCandidates,
  polygonArea,
  predictPosition,
  selectCoverMarkers,
  selectCoverPair,
  shotXg,
  shapeTriangle,
  spaceControl,
  splitUnits,
  threatGrammar,
  unitCentroid,
  visionCone,
  visionProfile,
  xtAt,
  xtGain,
} from './analysis.js';

const p = (x, y, role = ROLE.MID, shirt = 0, speed = 0) => ({
  x,
  y,
  role,
  shirt,
  speed,
});

test('formationLabel mirrors the engine shape labels', () => {
  const xi = (cats) => cats.map((c, i) => p(0, 0, c, i + 1));
  assert.equal(formationLabel(xi([1, 2, 2, 2, 2, 3, 3, 3, 4, 4, 4])), '4-3-3');
  assert.equal(formationLabel(xi([1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4])), '4-4-2');
  assert.equal(formationLabel(xi([1, 2, 2, 2, 2, 3, 3, 3, 3, 3, 4])), '4-2-3-1');
  assert.equal(formationLabel(xi([1, 2, 2, 2, 3, 3, 3, 3, 3, 4, 4])), '3-5-2');
});

test('detectMidfieldBank finds a settled, wide midfield line', () => {
  const mids = [
    p(-15, -20, ROLE.MID, 6),
    p(-15, -7, ROLE.MID, 7),
    p(-15, 7, ROLE.MID, 8),
    p(-15, 20, ROLE.MID, 10),
  ];
  const bank = detectMidfieldBank(mids);
  assert.ok(bank);
  assert.equal(bank.points.length, 4);
  assert.ok(bank.tightness > 0.9);
  // Points are sorted top-to-bottom in world (screen top-to-bottom after flip).
  assert.deepEqual(
    bank.points.map((m) => m.y),
    [20, 7, -7, -20],
  );
});

test('detectMidfieldBank drops a runner but keeps the remaining bank', () => {
  const broken = [
    p(-22, -12, ROLE.MID),
    p(-22, 0, ROLE.MID),
    p(-2, 12, ROLE.MID), // 20 m ahead of the bank
  ];
  const bank = detectMidfieldBank(broken);
  assert.ok(bank, 'the two settled midfielders still form a bank');
  assert.equal(bank.points.length, 2);
  assert.deepEqual(
    bank.points.map((m) => m.y),
    [0, -12],
  );
  assert.equal(detectMidfieldBanks(broken).length, 1);

  const cluster = [p(-15, -1, ROLE.MID), p(-15, 1, ROLE.MID)];
  assert.equal(detectMidfieldBank(cluster), null);

  assert.equal(detectMidfieldBank([p(-15, -5, ROLE.MID)]), null);
});

test('detectMidfieldBanks keeps a 4-2-3-1 double pivot and No.10 line', () => {
  const mids = [
    p(-18, -8, ROLE.MID, 6),
    p(-18, 8, ROLE.MID, 8),
    p(-6, -18, ROLE.MID, 7),
    p(-6, 0, ROLE.MID, 10),
    p(-6, 18, ROLE.MID, 11),
  ];
  const banks = detectMidfieldBanks(mids);
  assert.equal(banks.length, 2);
  assert.deepEqual(
    banks.map((b) => b.points.length),
    [2, 3],
  );
  assert.deepEqual(
    banks.map((b) => b.meanX),
    [-18, -6],
  );
  // The wrapper follows the largest bank, not the deepest one.
  assert.equal(detectMidfieldBank(mids).meanX, -6);
});

test('detectMidfieldBanks keeps a single 4-3-3 pivot line', () => {
  const mids = [
    p(-20, 0, ROLE.MID, 6),
    p(-14, -18, ROLE.MID, 8),
    p(-14, 18, ROLE.MID, 10),
  ];
  const banks = detectMidfieldBanks(mids);
  assert.equal(banks.length, 1);
  assert.equal(banks[0].points.length, 3);
  assert.equal(banks[0].meanX, -16);
});

test('attacking candidates prefer forwards and midfielders', () => {
  const team = [
    p(-45, 0, ROLE.GK, 1),
    p(-30, -10, ROLE.DEF, 2),
    p(-30, 10, ROLE.DEF, 3),
    p(-10, -5, ROLE.MID, 4),
    p(10, 0, ROLE.MID, 5),
    p(20, 5, ROLE.MID, 6),
  ];
  const attackers = attackingCandidates(team, 1);
  assert.deepEqual(
    attackers.map((x) => x.shirt),
    [6, 5, 4],
  );

  const noMidfield = [p(-45, 0, ROLE.GK, 1), p(-20, -5, ROLE.DEF, 2), p(15, 0, ROLE.DEF, 3)];
  assert.deepEqual(
    attackingCandidates(noMidfield, 1).map((x) => x.shirt),
    [3, 2],
  );
});

test('passLaneClear respects the clearance radius', () => {
  const ball = { x: 0, y: 0 };
  const threat = { x: 30, y: 0 };
  assert.equal(passLaneClear(ball, threat, [{ x: 15, y: 0 }], 3), false);
  assert.equal(passLaneClear(ball, threat, [{ x: 15, y: 6 }], 3), true);
  // Behind the ball or beyond the target does not block the lane.
  assert.equal(passLaneClear(ball, threat, [{ x: -8, y: 0 }], 3), true);
  assert.equal(passLaneClear(ball, threat, [{ x: 38, y: 0 }], 3), true);
});

test('evaluateThreats picks a free, advanced attacker', () => {
  const threatPlayer = p(30, 0, ROLE.FWD, 9, 5);
  const defenders = [p(40, -9, ROLE.DEF, 4), p(40, 9, ROLE.DEF, 5)];
  const ranked = evaluateThreats({
    attackers: [threatPlayer],
    cover: defenders,
    opponents: defenders,
    ball: { x: 10, y: 0 },
    dir: 1,
    offsideX: 40,
  });
  assert.equal(ranked.length, 1);
  assert.equal(ranked[0].player.shirt, 9);
  assert.ok(ranked[0].score >= ANALYSIS.threatMinScore);
  assert.equal(ranked[0].laneClear, true);
  assert.equal(ranked[0].cover.length, 2);
});

test('the danger zone uses the nearest marker on each flank', () => {
  const threat = p(30, 0, ROLE.FWD, 9);
  const goal = { x: 52.5, y: 0 };
  const pair = selectCoverPair(threat, goal, [
    p(33, 4, ROLE.DEF, 1),
    p(35, 6, ROLE.DEF, 2),
    p(33, -5, ROLE.DEF, 3),
    p(35, -7, ROLE.DEF, 4),
    p(40, 0.2, ROLE.DEF, 5), // on the shooting line, not a vertex
  ]);
  assert.deepEqual(
    pair.map((x) => x.shirt),
    [1, 3],
  );

  // Bracketing beats two closer markers on the same flank.
  const bracketed = selectCoverPair(threat, goal, [
    p(33, 1, ROLE.DEF, 1),
    p(33, 2, ROLE.DEF, 2),
    p(36, 8, ROLE.DEF, 3),
    p(36, -8, ROLE.DEF, 4),
  ]);
  assert.deepEqual(
    bracketed.map((x) => x.shirt),
    [1, 4],
  );

  // A single flank still yields a zone from the two nearest markers.
  assert.deepEqual(
    selectCoverPair(threat, goal, [p(33, 4, ROLE.DEF, 1), p(40, 0.2, ROLE.DEF, 2)]).map(
      (x) => x.shirt,
    ),
    [1, 2],
  );
  // A marker beyond the cover radius cannot be a vertex.
  assert.equal(selectCoverPair(threat, goal, [p(33, 4), p(33, -30)]), null);

  const opponents = [
    p(36, 0.2, ROLE.MID, 4), // directly in front, on the shooting line
    p(40, 9, ROLE.DEF, 5),
    p(40, 10, ROLE.DEF, 6),
  ];
  const ranked = evaluateThreats({
    attackers: [p(30, 0, ROLE.FWD, 9, 5)],
    cover: [p(29, -0.5, ROLE.MID, 4), ...opponents],
    opponents: [p(29, -0.5, ROLE.MID, 4), ...opponents],
    ball: { x: 10, y: 0 },
    dir: 1,
    offsideX: 40,
  });
  assert.equal(ranked.length, 0, 'under 2.2 m of space is too tight to be a threat');

  const open = evaluateThreats({
    attackers: [p(30, 0, ROLE.FWD, 9, 5)],
    cover: opponents,
    opponents,
    ball: { x: 10, y: 0 },
    dir: 1,
    offsideX: 40,
  });
  assert.equal(open.length, 1);
  // No marker on the other flank: fall back to the two nearest goal-side
  // markers, even though they are on the same side.
  assert.deepEqual(
    open[0].cover.map((x) => x.shirt),
    [4, 5],
  );
});

test('evaluateThreats rejects offside and tightly marked attackers', () => {
  const attackers = [p(30, 0, ROLE.FWD, 9, 5)];
  const defenders = [p(18, -9, ROLE.DEF, 4), p(18, 9, ROLE.DEF, 5)];

  const offside = evaluateThreats({
    attackers,
    cover: defenders,
    opponents: defenders,
    ball: { x: 10, y: 0 },
    dir: 1,
    offsideX: 25,
  });
  assert.equal(offside.length, 0);

  const marked = evaluateThreats({
    attackers,
    cover: [p(29, 0, ROLE.DEF, 4), p(18, 9, ROLE.DEF, 5)],
    opponents: [p(29, 0, ROLE.DEF, 4), p(18, 9, ROLE.DEF, 5)],
    ball: { x: 10, y: 0 },
    dir: 1,
    offsideX: 40,
  });
  assert.equal(marked.length, 0);
});

test('the away threat is evaluated toward the −x goal', () => {
  const attacker = p(-30, 0, ROLE.FWD, 11, 4);
  const defenders = [p(-40, -9, ROLE.DEF, 4), p(-40, 9, ROLE.DEF, 5)];
  const ranked = evaluateThreats({
    attackers: [attacker],
    cover: defenders,
    opponents: defenders,
    ball: { x: -10, y: 0 },
    dir: -1,
    offsideX: -40,
  });
  assert.equal(ranked.length, 1);
  assert.equal(ranked[0].player.shirt, 11);
  assert.equal(ranked[0].cover.length, 2);
});

test('buildDangerZone produces a compact triangle from real vertices', () => {
  const threat = {
    player: p(30, 0, ROLE.FWD, 9),
    cover: [p(18, -8, ROLE.DEF, 4), p(18, 8, ROLE.DEF, 5)],
  };
  const zone = buildDangerZone(threat);
  assert.ok(zone);
  assert.equal(zone.points.length, 3);
  assert.ok(zone.area >= ANALYSIS.zoneMinArea && zone.area <= ANALYSIS.zoneMaxArea);
  assert.ok(zone.maxEdge <= ANALYSIS.zoneMaxEdge);
  // All three original players are vertices.
  for (const player of [threat.player, ...threat.cover]) {
    assert.ok(zone.points.some((v) => v.x === player.x && v.y === player.y));
  }
});

test('selectCoverMarkers spreads the zone across angular sectors', () => {
  const threat = p(30, 0, ROLE.FWD, 9);
  const goal = { x: 52.5, y: 0 };
  const markers = selectCoverMarkers(threat, goal, [
    p(36, -14, ROLE.DEF, 1),
    p(39, -5, ROLE.DEF, 2),
    p(39, 5, ROLE.DEF, 3),
    p(36, 14, ROLE.DEF, 4),
  ]);
  assert.deepEqual(
    markers.map((x) => x.shirt),
    [1, 2, 3, 4],
  );

  // One occupied sector still yields the two nearest markers for a polygon.
  const singleSector = selectCoverMarkers(threat, goal, [
    p(38, -2, ROLE.DEF, 1),
    p(40, -3, ROLE.DEF, 2),
  ]);
  assert.deepEqual(
    singleSector.map((x) => x.shirt),
    [1, 2],
  );
});

test('buildDangerZone uses a 4-6 vertex hull when the markers warrant it', () => {
  const threat = {
    player: p(30, 0, ROLE.FWD, 9),
    cover: [
      p(36, -14, ROLE.DEF, 1),
      p(39, -5, ROLE.DEF, 2),
      p(39, 5, ROLE.DEF, 3),
      p(36, 14, ROLE.DEF, 4),
    ],
  };
  const zone = buildDangerZone(threat);
  assert.ok(zone);
  assert.equal(zone.points.length, 5);
  assert.equal(zone.vertices, 5);
  assert.ok(zone.area >= ANALYSIS.zoneMinArea && zone.area <= ANALYSIS.zoneMaxArea);
  assert.ok(zone.maxEdge <= ANALYSIS.zoneMaxEdge);
  for (const player of [threat.player, ...threat.cover]) {
    assert.ok(zone.points.some((v) => v.x === player.x && v.y === player.y));
  }
});

test('buildDangerZone falls back to a triangle when the hull is too large', () => {
  const threat = {
    player: p(30, 0, ROLE.FWD, 9),
    cover: [
      p(41, -19, ROLE.DEF, 1),
      p(50.7, -7.5, ROLE.DEF, 2),
      p(50.7, 7.5, ROLE.DEF, 3),
      p(41, 19, ROLE.DEF, 4),
    ],
  };
  const zone = buildDangerZone(threat);
  assert.ok(zone);
  assert.equal(zone.points.length, 3);
  assert.ok(zone.area <= ANALYSIS.zoneTriangleMaxArea);
  assert.ok(zone.maxEdge <= ANALYSIS.zoneTriangleMaxEdge);
});

test('buildDangerZone rejects degenerate and oversized triangles', () => {
  const collinear = {
    player: p(30, 0),
    cover: [p(20, 0), p(10, 0)],
  };
  assert.equal(buildDangerZone(collinear), null);

  const oversized = {
    player: p(45, 0),
    cover: [p(5, -20), p(5, 20)],
  };
  assert.equal(buildDangerZone(oversized), null);

  assert.equal(buildDangerZone({ player: p(30, 0), cover: [] }), null);
});

test('convexHull and polygonArea are stable for small point sets', () => {
  const hull = convexHull([
    { x: 0, y: 0 },
    { x: 10, y: 0 },
    { x: 0, y: 10 },
    { x: 5, y: 5 }, // inside
    { x: 0, y: 0 }, // duplicate
  ]);
  assert.equal(hull.length, 3);
  assert.equal(polygonArea(hull), 50);
  assert.equal(polygonArea([]), 0);
  assert.equal(polygonArea([{ x: 0, y: 0 }, { x: 1, y: 1 }]), 0);
});

test('analysis maths never emits non-finite geometry', () => {
  const zone = buildDangerZone({
    player: p(30, 0, ROLE.FWD, 9),
    cover: [p(20, -10, ROLE.DEF, 4), p(20, 10, ROLE.DEF, 5)],
  });
  assert.ok(zone);
  for (const v of zone.points) {
    assert.ok(Number.isFinite(v.x) && Number.isFinite(v.y));
  }
});

test('xtAt peaks centrally in the final third', () => {
  const box = xtAt(40, 0, 1);
  const own = xtAt(-40, 0, 1);
  const wide = xtAt(40, 30, 1);
  assert.ok(box > own, `box ${box} should beat own half ${own}`);
  assert.ok(box > wide, `central ${box} should beat wide ${wide}`);
  assert.ok(box >= 0 && box <= 1);
  assert.ok(xtGain({ x: 0, y: 0 }, { x: 40, y: 0 }, 1) > 0);
});

test('shotXg falls with distance', () => {
  const near = shotXg(45, 0, 1);
  const far = shotXg(10, 0, 1);
  assert.ok(near > far, `near ${near} should beat far ${far}`);
  assert.ok(near <= 0.65 && far >= 0.02);
});

test('passFan proposes calibrated corridors and rejects blocked lanes', () => {
  const ball = { x: 0, y: 0 };
  const open = { x: 20, y: 0, vx: 0, vy: 0, speed: 0 };
  const marked = { x: 20, y: 10, vx: 0, vy: 0, speed: 0 };
  const blocker = { x: 10, y: 0, vx: 0, vy: 0 };
  const fan = passFan(ball, [open, marked], [], 1);
  assert.equal(fan.length, 2);
  assert.ok(fan[0].p >= 0 && fan[0].p <= 1);
  assert.ok(fan[0].clearance > 0);
  // A defender sitting on the lane kills that option.
  const blocked = passFan(ball, [open], [blocker], 1);
  assert.equal(blocked.length, 0);
});

test('corridorClearance measures the nearest defender', () => {
  const { clearance, beats } = corridorClearance(
    { x: 0, y: 0 },
    { x: 20, y: 0 },
    [{ x: 10, y: 3, vx: 0, vy: 0 }],
    1.0,
  );
  assert.ok(Math.abs(clearance - 3) < 0.6, `clearance ${clearance}`);
  assert.equal(beats, true);
});

test('spaceControl shrinks under pressure', () => {
  const threat = { player: { x: 30, y: 0, speed: 5 } };
  const free = spaceControl(threat, [{ x: 0, y: 0, vx: 0, vy: 0 }]);
  const pressed = spaceControl(threat, [{ x: 31, y: 0, vx: 5, vy: 0 }]);
  assert.ok(free.area > 0);
  assert.ok(pressed.area < free.area, `${pressed.area} should be < ${free.area}`);
});

test('backLineStats reports height, width and gaps', () => {
  const defs = [
    { x: -30, y: -12, role: ROLE.DEF },
    { x: -30, y: -4, role: ROLE.DEF },
    { x: -30, y: 4, role: ROLE.DEF },
    { x: -30, y: 12, role: ROLE.DEF },
  ];
  const mids = [
    { x: -15, y: -8, role: ROLE.MID },
    { x: -15, y: 8, role: ROLE.MID },
  ];
  const fwds = [{ x: 0, y: 0, role: ROLE.FWD }];
  const st = backLineStats([...defs, ...mids, ...fwds], 1);
  assert.ok(Math.abs(st.height + 30) < 0.01);
  assert.ok(Math.abs(st.width - 24) < 0.01);
  assert.ok(st.maxGap >= 8);
  assert.ok(st.defMidGap > 10);
});

test('threatGrammar classifies shot, cross, through and feet', () => {
  const ball = { x: 30, y: 0 };
  assert.equal(threatGrammar({ player: { x: 40, y: 0 } }, ball, [], 1).kind, 'shot');
  const wide = threatGrammar({ player: { x: 30, y: 20 } }, { x: 35, y: 25 }, [], 1);
  assert.equal(wide.kind, 'cross');
  assert.equal(wide.zone.length, 3);
  const through = threatGrammar({ player: { x: 10, y: 0 } }, { x: 0, y: 0 }, [{ x: 20, y: 0 }], 1);
  assert.equal(through.kind, 'through');
  const feet = threatGrammar({ player: { x: 10, y: 0 } }, { x: 0, y: 0 }, [], 1);
  assert.equal(feet.kind, 'feet');
  assert.deepEqual(crossZone({ x: 30, y: 20 }, 1).length, 3);
});

test('predictPosition projects velocity', () => {
  const q = predictPosition({ x: 0, y: 0, vx: 4, vy: -2 }, 1.5);
  assert.ok(Math.abs(q.x - 6) < 1e-9 && Math.abs(q.y + 3) < 1e-9);
});

test('splitUnits buckets outfield roles and excludes the keeper', () => {
  const team = [
    p(-45, 0, ROLE.GK, 1),
    p(-30, -10, ROLE.DEF, 2),
    p(-30, 10, ROLE.DEF, 3),
    p(-10, 0, ROLE.MID, 6),
    p(20, 4, ROLE.FWD, 9),
  ];
  const units = splitUnits(team);
  assert.equal(units.def.length, 2);
  assert.equal(units.mid.length, 1);
  assert.equal(units.fwd.length, 1);
  assert.ok(units.def.every((q) => q.role === ROLE.DEF));
  // Top-to-bottom in world y (screen order after the flip).
  assert.deepEqual(units.def.map((q) => q.y), [10, -10]);
});

test('unit centroids drive the offensive triangle', () => {
  const units = splitUnits([
    p(-30, -10, ROLE.DEF, 2),
    p(-30, 10, ROLE.DEF, 3),
    p(-10, -5, ROLE.MID, 6),
    p(-10, 5, ROLE.MID, 8),
    p(20, 0, ROLE.FWD, 9),
  ]);
  const def = unitCentroid(units.def);
  const mid = unitCentroid(units.mid);
  assert.ok(Math.abs(def.x + 30) < 1e-9 && Math.abs(def.y) < 1e-9);
  assert.ok(Math.abs(mid.x + 10) < 1e-9 && Math.abs(mid.y) < 1e-9);
  const tri = shapeTriangle(units);
  assert.deepEqual(Object.keys(tri).sort(), ['def', 'fwd', 'mid']);
  assert.ok(Math.abs(tri.fwd.x - 20) < 1e-9);
});

test('shapeTriangle is null unless all three units exist', () => {
  assert.equal(unitCentroid([]), null);
  const noForward = splitUnits([
    p(-30, 0, ROLE.DEF, 2),
    p(-10, 0, ROLE.MID, 6),
  ]);
  assert.equal(shapeTriangle(noForward), null);
});

test('splitUnits falls back to attacking depth when roles are missing', () => {
  const team = [
    p(-45, 0, ROLE.GK, 1),
    ...Array.from({ length: 10 }, (_, i) =>
      p(-40 + i * 8, i % 2 ? 8 : -8, ROLE.UNKNOWN, i + 2)),
  ];
  const units = splitUnits(team, 1);
  assert.equal(units.def.length, 4);
  assert.equal(units.mid.length, 3);
  assert.equal(units.fwd.length, 3);
  assert.ok(units.def.every((q) => q.role !== ROLE.GK));
  // Depth order is respected: deepest band first.
  const maxDef = Math.max(...units.def.map((q) => q.x));
  const minMid = Math.min(...units.mid.map((q) => q.x));
  const minFwd = Math.min(...units.fwd.map((q) => q.x));
  assert.ok(maxDef < minMid && minMid < minFwd);
});

test('applyRoles writes the cached roles onto fresh snapshots', () => {
  const players = [{ role: ROLE.UNKNOWN }, { role: ROLE.UNKNOWN }, {}];
  applyRoles(players, [ROLE.DEF, ROLE.FWD]);
  assert.equal(players[0].role, ROLE.DEF);
  assert.equal(players[1].role, ROLE.FWD);
  assert.equal(players[2].role, ROLE.UNKNOWN);
});

test('clamp01 treats non-finite input as zero', () => {
  assert.equal(clamp01(NaN), 0);
  assert.equal(clamp01(Infinity), 1);
  assert.equal(clamp01(-Infinity), 0);
  assert.equal(clamp01(0.4), 0.4);
});

test('visionCone classifies visible and blind teammates inside its radius', () => {
  const cone = visionCone(
    { x: 0, y: 0, heading: 0 },
    [
      { x: 8, y: 1, shirt: 8 },   // inside the cone
      { x: -8, y: 1, shirt: 9 },  // behind the player
      { x: 30, y: 0, shirt: 10 }, // beyond the 22 m radius
    ],
    [],
    1,
  );
  assert.equal(cone.visible.length, 1);
  assert.equal(cone.visible[0].shirt, 8);
  // The 30 m teammate is outside the vision radius and is not linked.
  assert.equal(cone.blind.length, 1);
  assert.equal(cone.blind[0].shirt, 9);
  assert.ok(cone.conePoints.length > 3);
  assert.ok(cone.blindConePoints.length > 3);
  assert.ok(cone.coverage > 0 && cone.coverage < 1);
});

test('vision profiles vary per player and stay deterministic', () => {
  const a = visionProfile({ team: 0, shirt: 9 });
  const b = visionProfile({ team: 0, shirt: 4 });
  const c = visionProfile({ team: 1, shirt: 9 });
  assert.deepEqual(visionProfile({ team: 0, shirt: 9 }), a);
  assert.ok(a.radius >= 16 && a.radius <= 28);
  assert.ok(a.halfAngle >= (48 * Math.PI) / 180);
  assert.ok(a.halfAngle <= (72 * Math.PI) / 180);
  assert.ok(a.radius !== b.radius || a.halfAngle !== b.halfAngle);
  assert.ok(a.radius !== c.radius || a.halfAngle !== c.halfAngle);
  const cone = visionCone({ x: 0, y: 0, heading: 0, team: 0, shirt: 9 }, [], [], 1);
  assert.equal(cone.radius, a.radius);
  assert.equal(cone.halfAngle, a.halfAngle);
});

test('visionCone treats a teammate behind an opponent as blind', () => {
  const cone = visionCone(
    { x: 0, y: 0, heading: 0 },
    [{ x: 10, y: 0, shirt: 8 }],
    [{ x: 5, y: 0, shirt: 5 }],
    1,
  );
  assert.equal(cone.visible.length, 0);
  assert.equal(cone.blind.length, 1);
});

test('visionCone faces the best passing focus instead of the ball', () => {
  const cone = visionCone(
    { x: 0, y: 0, heading: 0, team: 0, shirt: 9 },
    [{ x: 0, y: -8, shirt: 8 }, { x: 8, y: 0, shirt: 10 }],
    [],
    1,
    { x: 0, y: -12 },
  );
  assert.ok(Math.abs(cone.facing + Math.PI / 2) < 1e-9);
  assert.equal(cone.visible[0].shirt, 8);
  assert.equal(cone.blind[0].shirt, 10);
});

test('visionCone falls back to velocity when heading is missing', () => {
  const cone = visionCone({ x: 0, y: 0, vx: 0, vy: 6 }, [{ x: 0, y: 10 }], [], 1);
  assert.ok(Math.abs(cone.facing - Math.PI / 2) < 1e-9);
  assert.equal(cone.visible.length, 1);
});

test('attackChannels proposes meaningful forward runs', () => {
  const players = [
    { x: 20, y: 0, team: 0, role: ROLE.FWD, shirt: 9, speed: 7 },
    { x: 10, y: -10, team: 0, role: ROLE.MID, shirt: 8, speed: 6 },
    { x: -20, y: 0, team: 1, role: ROLE.DEF, shirt: 4, speed: 4 },
    { x: -10, y: 5, team: 1, role: ROLE.DEF, shirt: 5, speed: 4 },
  ];
  const channels = attackChannels(players, { x: 16, y: 0 }, 0, 1, players.filter((q) => q.team === 1));
  assert.ok(channels.length > 0 && channels.length <= 3);
  for (const ch of channels) {
    const len = Math.hypot(ch.to.x - ch.from.x, ch.to.y - ch.from.y);
    assert.ok(len >= 5, `run ${len} should be a real movement arrow`);
    assert.ok(len <= 13, `run ${len} should stay short and readable`);
    assert.ok(ch.score >= 0 && ch.score <= 1);
    assert.ok(Number.isFinite(ch.to.x) && Number.isFinite(ch.to.y));
  }
  assert.equal(attackChannels(players, { x: -10, y: 0 }, 0, 1, []).length, 0);
});

test('attackChannels bows the run away from a defender on the line', () => {
  const players = [
    { x: 24, y: 0, team: 0, role: ROLE.FWD, shirt: 9, speed: 7 },
    { x: 30, y: 0, team: 1, role: ROLE.DEF, shirt: 4, speed: 4 },
  ];
  const channels = attackChannels(players, { x: 16, y: 0 }, 0, 1, [players[1]]);
  assert.equal(channels.length, 1);
  assert.ok(Math.abs(channels[0].curve) > 1, 'the bowed route avoids the defender');
  assert.ok(Math.abs(channels[0].curve) <= 4);
  const len = Math.hypot(
    channels[0].to.x - channels[0].from.x,
    channels[0].to.y - channels[0].from.y,
  );
  assert.ok(len >= 5 && len <= 13);
});

test('passingLaneCandidates returns scored lanes from the carrier', () => {
  const players = [
    { x: 0, y: 0, team: 0, role: ROLE.MID, shirt: 6, vx: 0, vy: 0 },
    { x: 15, y: 0, team: 0, role: ROLE.FWD, shirt: 9, vx: 0, vy: 0 },
    { x: 0, y: 20, team: 1, role: ROLE.DEF, shirt: 4, vx: 0, vy: 0 },
  ];
  const lanes = passingLaneCandidates(players, { x: 0.5, y: 0 }, 0, 1);
  assert.equal(lanes.length, 1);
  assert.equal(lanes[0].player.shirt, 9);
  assert.ok(lanes[0].p >= 0 && lanes[0].p <= 1);
  assert.ok(Number.isFinite(lanes[0].clearance));
});
