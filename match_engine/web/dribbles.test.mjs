import test from 'node:test';
import assert from 'node:assert/strict';

import {
  EFFECTIVENESS,
  ZONE_COLS,
  ZONE_ROWS,
  createDribbleIndex,
  effectivenessLabel,
  formatClock,
  zoneLabel,
  zoneOf,
  zoneWorldRect,
} from './dribbles.js';

/** Build one analysis-log row: [ty, t, mt, period, team, pid, name, x, y, ex, ey, out]. */
function row(ty, t, team, pid, name, x, y, opts = {}) {
  return [
    ty,
    opts.t ?? t,
    opts.mt ?? t,
    opts.period ?? 1,
    team,
    pid,
    name,
    x,
    y,
    opts.ex ?? null,
    opts.ey ?? null,
    opts.out ?? 1,
  ];
}

const HOME_ROSTER = [
  ['h7', 'Seven', 7, 3],
  ['h8', 'Eight', 8, 3],
  ['h9', 'Nine', 9, 4],
  ['h10', 'J. Smith', 10, 4],
  ['h11', 'Eleven', 11, 2],
];
const AWAY_ROSTER = [['a9', 'Away Nine', 9, 4]];

function fixture() {
  const events = [
    // h10: six dribbles in the final third (two won), then three safer ones.
    row(3, 300, 0, 'h10', 'J. Smith', 40, 30),
    row(3, 320, 0, 'h10', 'J. Smith', 45, -20, { out: 0 }),
    row(3, 340, 0, 'h10', 'J. Smith', 30, 10, { out: 0 }),
    row(3, 360, 0, 'h10', 'J. Smith', 25, -30, { out: 0 }),
    row(3, 380, 0, 'h10', 'J. Smith', 50, 5),
    row(3, 400, 0, 'h10', 'J. Smith', 20, 25, { out: 0 }),
    row(16, 500, 1, 'a9', 'Away Nine', 10, 5), // away score first
    row(16, 505, 0, 'h9', 'Nine', 20, 0), // home level again
    row(16, 510, 0, 'h9', 'Nine', 30, 0), // then ahead
    row(83, 600, 0, 'h10', 'J. Smith', 10, 0, { ex: 25, ey: 0 }), // carry +15 m
    row(1, 605, 0, 'h10', 'J. Smith', 25, 0), // keeps possession after it
    row(3, 620, 0, 'h10', 'J. Smith', 0, 0),
    row(3, 640, 0, 'h10', 'J. Smith', -30, 0),
    // A shot five seconds after the first take-on (type 13 = miss).
    row(13, 305, 0, 'h10', 'J. Smith', 45, 30),
    // Team-mates with small dribble samples (percentile benchmarks).
    row(3, 700, 0, 'h9', 'Nine', -40, 0, { ex: -25, ey: 0 }),
    row(3, 720, 0, 'h9', 'Nine', -20, 10),
    row(3, 740, 0, 'h11', 'Eleven', -10, -10, { ex: 0, ey: -10 }),
    row(3, 760, 0, 'h11', 'Eleven', -25, 5, { out: 0 }),
    row(3, 780, 0, 'h8', 'Eight', 5, 20, { out: 0, ex: 15, ey: 20 }),
    row(3, 800, 0, 'h8', 'Eight', 40, 15, { out: 0 }),
    // One second-half dribble for the period filter.
    row(3, 3000, 0, 'h10', 'J. Smith', 30, 0, { period: 2, mt: 5000 }),
    // Meta rows the engine would already have dropped — must be ignored.
    row(34, 0, 0, '', '', 0, 0),
  ];
  return createDribbleIndex({ v: 1, events, roster: [HOME_ROSTER, AWAY_ROSTER] });
}

test('zone geometry is 12 zones in the attacker’s own frame', () => {
  // Home: world == attacking frame.
  assert.equal(zoneOf(40, 30, 0), 3, 'final third, left flank');
  assert.equal(zoneOf(-40, -30, 0), 10, 'defensive third, right flank');
  // Away mirrors both axes: the same attacking spot maps to the same zone.
  assert.equal(zoneOf(-40, -30, 1), 3, 'away attacking position mirrors');
  assert.equal(zoneOf(52.5, -34, 1), 1, 'their own-goal corner is their defensive left');
  // Corners stay inside the grid.
  assert.equal(zoneOf(52.5, 34, 0), 3);
  assert.equal(zoneOf(-52.5, -34, 0), 10);
  assert.ok(ZONE_COLS * ZONE_ROWS === 12);
});

test('zone world rectangles mirror back for the away team', () => {
  const homeFinalLeft = zoneWorldRect(3, 0);
  assert.deepEqual(homeFinalLeft, { x0: 17.5, y0: 17, x1: 52.5, y1: 34 });
  const awayFinalLeft = zoneWorldRect(3, 1);
  assert.deepEqual(awayFinalLeft, { x0: -52.5, y0: -34, x1: -17.5, y1: -17 });
  // Every zone covers exactly one twelfth of the pitch.
  for (let z = 1; z <= 12; z++) {
    const r = zoneWorldRect(z, 0);
    assert.ok(Math.abs((r.x1 - r.x0) * (r.y1 - r.y0) - (105 * 68) / 12) < 1e-9);
  }
  assert.equal(zoneLabel(3), 'left flank · final third');
  assert.equal(zoneLabel(10), 'right flank · defensive third');
});

test('the index extracts take-ons and carries only, with forward gain', () => {
  const index = fixture();
  // 11 h10/h9/h11/h8 first-half dribbles + the second-half one; type 34 ignored.
  assert.equal(index.dribbles.length, 16);
  const carry = index.dribbles.find((d) => d.kind === 'carry');
  assert.ok(carry, 'type 83 counted as a carry');
  assert.equal(carry.gain, 15, 'end point projects to +15 m forward');
  const takeOn = index.dribbles.find((d) => d.kind === 'takeon');
  assert.equal(takeOn.gain, null, 'take-ons without an end point report no gain');
  assert.equal(index.rosterByPid.get('h10').name, 'J. Smith');
  assert.equal(index.playerByKey(0, 10).pid, 'h10');
  assert.equal(index.playerByKey(1, 9).pid, 'a9');
});

test('post-dribble outcomes look at the player’s next action within 8 s', () => {
  const index = fixture();
  const mine = index.dribblesOf('h10');
  const first = mine.find((d) => d.t === 300);
  assert.equal(first.shotAfter, true, 'goal at +5 s counts as a created shot');
  const carry = mine.find((d) => d.kind === 'carry');
  assert.equal(carry.keepAfter, true, 'completed pass at +5 s keeps the move alive');
  assert.equal(carry.shotAfter, null);
  const last = mine[mine.length - 1];
  assert.equal(last.shotAfter, null, 'no action inside the window');
});

test('player stats split by third, zone and outcome', () => {
  const index = fixture();
  const stats = index.playerStats('h10');
  assert.equal(stats.attempts, 10);
  assert.equal(stats.successes, 6);
  assert.ok(Math.abs(stats.success - 0.6) < 1e-9);
  assert.equal(stats.takeons, 9);
  assert.equal(stats.takeonWins, 5);
  assert.equal(stats.lost, 4);
  assert.equal(stats.shotAfter, 1);
  assert.equal(stats.keepAfter, 1);
  assert.equal(stats.gainTotal, 15);
  assert.equal(stats.avgGain, 15);

  const [def, mid, fin] = stats.thirds;
  assert.equal(def.attempts, 1);
  assert.equal(mid.attempts, 2);
  assert.equal(fin.attempts, 7, 'six first-half plus the second-half carry');
  assert.equal(fin.successes, 3);

  const zone3 = stats.zones[2];
  assert.equal(zone3.attempts, 2);
  assert.equal(zone3.successes, 1);
});

test('filters split by period and by game state', () => {
  const index = fixture();
  const h10 = index.playerStats('h10', { period: 1 });
  assert.equal(h10.attempts, 9, 'second-half dribble excluded');
  const h2 = index.playerStats('h10', { period: 2 });
  assert.equal(h2.attempts, 1);

  // Home trail 0-1 from 500 s, level again from 505 s.
  assert.equal(index.gameState(400, 0), 'level');
  assert.equal(index.gameState(502, 0), 'behind');
  assert.equal(index.gameState(600, 0), 'ahead');
  assert.equal(index.gameState(502, 1), 'ahead');
  const level = index.teamDribbles(0, { state: 'level' });
  const ahead = index.teamDribbles(0, { state: 'ahead' });
  assert.equal(level.length + ahead.length, index.teamDribbles(0).length);
  assert.ok(level.every((d) => d.mt < 500), 'level-state dribbles all pre-goal');
  const onlyH10 = index.teamDribbles(0, { player: 'h10' });
  assert.equal(onlyH10.length, 10);
});

test('percentiles benchmark a player against team-mates with sample', () => {
  const index = fixture();
  const pct = index.playerPercentiles('h10');
  assert.equal(pct.mateCount, 4, 'h7 has no dribbles and is not benchmarked');
  assert.equal(pct.attempts, 75, 'three of four team-mates attempted fewer');
  assert.equal(pct.success, 50);
  assert.equal(pct.gain, 50, 'two of four team-mates progressed further');
  assert.equal(index.playerPercentiles('h7').attempts, null, 'no attempts → no percentile');

  assert.equal(index.stars(45), 3);
  assert.equal(index.stars(76), 4);
  assert.equal(index.stars(82), 4);
  assert.equal(index.stars(91), 5);
  assert.equal(index.stars(null), 0);
});

test('insights flag the final-third concern and stay quiet elsewhere', () => {
  const index = fixture();
  const stats = index.playerStats('h10');
  const notes = index.insights(stats, 'J. Smith');
  const concern = notes.find((n) => n.kind === 'concern');
  assert.ok(concern, 'a low final-third rate with volume must surface');
  assert.match(concern.title, /final third/);
  assert.match(concern.body, /43% success on 7 attempts/);
  assert.equal(notes.find((n) => n.kind === 'strength'), undefined);
  assert.equal(notes.find((n) => n.kind === 'opportunity'), undefined);

  // A player with no dribbles gets direction, not a shrug.
  const empty = index.insights(index.playerStats('h7'), 'Seven');
  assert.equal(empty.length, 1);
  assert.equal(empty[0].kind, 'empty');
});

test('the effectiveness scale matches the brief’s bands', () => {
  assert.equal(effectivenessLabel(95), 'Elite');
  assert.equal(effectivenessLabel(75), 'Very good');
  assert.equal(effectivenessLabel(60), 'Good');
  assert.equal(effectivenessLabel(45), 'Average');
  assert.equal(effectivenessLabel(30), 'Below average');
  assert.equal(effectivenessLabel(15), 'Poor');
  assert.equal(effectivenessLabel(0), 'Critical');
  assert.equal(EFFECTIVENESS.length, 7);
});

test('clock formatting pads to mm:ss', () => {
  assert.equal(formatClock(0), '00:00');
  assert.equal(formatClock(309.9), '05:09');
  assert.equal(formatClock(5400), '90:00');
});

test('a malformed log is rejected instead of half-parsed', () => {
  assert.throws(() => createDribbleIndex({ v: 2, events: [] }), /schema v1/);
  assert.throws(() => createDribbleIndex(null), /schema v1/);
});

test('anonymous roster players are labelled by shirt, never by feed id', () => {
  const roster = [
    [['h10', 'J. Smith', 10, 4], ['h49', '', 49, 5]],
    [['a9', 'Away Nine', 9, 4], ['a0', '', 0, 5]],
  ];
  const index = createDribbleIndex({
    v: 1,
    events: [row(3, 300, 0, 'h49', '', 40, 30)],
    roster,
  });
  assert.equal(index.rosterByPid.get('h49').name, '', 'the raw feed stays empty');
  assert.equal(index.rosterByPid.get('h49').label, 'Player 49');
  assert.equal(index.playerByKey(0, 49).label, 'Player 49');
  assert.equal(index.playerByKey(1, 0).label, 'Unnamed player', 'no shirt, no hash');
  // Events without a name inherit the roster label rather than an id.
  assert.equal(index.dribbles[0].name, 'Player 49');
});

test('a feed without take-on or carry events is flagged honestly', () => {
  const index = fixture();
  assert.equal(index.hasDribbleData, true);
  const passes = createDribbleIndex({
    v: 1,
    events: [row(1, 100, 0, 'h10', 'J. Smith', 0, 0, { ex: 10, ey: 0 })],
    roster: [HOME_ROSTER, []],
  });
  assert.equal(passes.hasDribbleData, false);
  assert.equal(passes.dribbles.length, 0);
});

test('the horizon snaps to the stat boundaries a dribble creates', () => {
  const index = fixture();
  assert.equal(index.horizonAt(null), null, 'no horizon = whole match');
  assert.equal(index.horizonAt(Infinity), null);
  assert.equal(index.horizonAt(0), -1, 'before the first dribble nothing counts');
  assert.equal(index.horizonAt(299.9), -1);
  assert.equal(index.horizonAt(300), 300);
  assert.equal(index.horizonAt(305), 300, 'inside an outcome window, nothing changed');
  assert.equal(index.horizonAt(308), 308, 'the window closed');
  assert.equal(index.horizonAt(99_999), 5008, 'far ahead = last outcome boundary');
});

test('the live horizon filters stats and never leaks future outcomes', () => {
  const index = fixture();
  assert.equal(index.playerStats('h10', { until: 350 }).attempts, 3);
  assert.equal(index.playerStats('h10', { until: 339 }).attempts, 2);
  // At 303 s the first take-on exists, but its shot (305 s) is still future.
  const early = index.playerStats('h10', { until: 303 });
  assert.equal(early.attempts, 1);
  assert.equal(early.shotAfter, 0, 'the +5 s shot is not visible yet');
  assert.equal(early.dribbles[0].shotAfter, null, 'the flag is suppressed, not faked');
  const settled = index.playerStats('h10', { until: 308 });
  assert.equal(settled.shotAfter, 1, 'the shot is counted once its window closes');
  assert.equal(index.teamStats(0, { until: 400 }).attempts, 6);
  assert.equal(index.teamStats(1, { until: 400 }).attempts, 0);
});

test('horizon snapping keeps the stat cache bounded and consistent', () => {
  const index = fixture();
  const a = index.playerStats('h10', { until: 303 });
  const b = index.playerStats('h10', { until: 305 });
  assert.equal(a, b, 'equal snapped horizons share one cached object');
  assert.notEqual(a, index.playerStats('h10', { until: 308 }));
  assert.notEqual(
    index.playerStats('h10', { until: 308 }),
    index.playerStats('h10'),
    'a finite horizon never collides with whole-match stats',
  );
});

test('game state is judged at the dribble clock, not the horizon', () => {
  const index = fixture(); // goals at 500 (away), 505 and 510 (home)
  assert.equal(index.gameState(400, 0), 'level');
  // A horizon before the goals still sees only level dribbles, even though
  // the match later ends with the home side ahead.
  assert.equal(index.teamStats(0, { state: 'level', until: 504 }).attempts, 6);
  assert.equal(index.teamStats(0, { state: 'ahead', until: 504 }).attempts, 0);
  assert.equal(index.teamStats(0, { state: 'ahead', until: 600 }).attempts, 1);
});

test('percentile benchmarks use the same live window as the player', () => {
  const index = fixture();
  const early = index.playerPercentiles('h10', { until: 340 });
  assert.equal(early.mateCount, 1, 'only h10 has 2+ dribbles at 05:40');
  assert.equal(early.attempts, null, 'no benchmark, no percentile — not a fake 0');
  const full = index.playerPercentiles('h10');
  assert.ok(full.mateCount >= 3);
  assert.notEqual(full.attempts, null);
});
