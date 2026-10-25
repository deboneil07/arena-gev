import test from 'node:test';
import assert from 'node:assert/strict';

import {
  PAVILION,
  createSubstitutionTracker,
  substitutionPath,
} from './transitions.js';

const close = (a, b, eps = 1e-6) => Math.abs(a - b) <= eps;

function squad() {
  return [
    { team: 0, shirt: 6, x: 10, y: -4 },
    { team: 0, shirt: 9, x: 20, y: 10 },
    { team: 1, shirt: 4, x: -18, y: 2 },
  ];
}

test('substitutionPath eases from the old body to the pavilion', () => {
  const from = { x: 10, y: -4 };
  const to = { x: -14, y: 36.2 };
  const start = substitutionPath(from, to, 0);
  const mid = substitutionPath(from, to, 0.5);
  const end = substitutionPath(from, to, 1);

  assert.ok(close(start.x, from.x) && close(start.y, from.y));
  assert.ok(close(end.x, to.x) && close(end.y, to.y));
  assert.ok(mid.x < from.x && mid.x > to.x);
  assert.ok(mid.y > from.y && mid.y < to.y);
  // Beyond the endpoints never overshoots.
  assert.ok(close(substitutionPath(from, to, -2).x, from.x));
  assert.ok(close(substitutionPath(from, to, 2).x, to.x));
});

test('the first snapshot primes the tracker without a ghost', () => {
  const tracker = createSubstitutionTracker();
  const players = squad();
  const items = tracker.update(players, 1 / 60);
  assert.deepEqual(items, []);
  assert.equal(tracker.activeCount, 0);
  assert.deepEqual([...tracker.hiddenSlots()], []);

  // Ordinary movement must not be mistaken for a substitution.
  players[0].x += 1.5;
  players[0].y -= 0.4;
  assert.deepEqual(tracker.update(players, 1 / 60), []);
});

test('a shirt change spawns a walk-off and a walk-on', () => {
  const tracker = createSubstitutionTracker();
  const players = squad();
  tracker.update(players, 0);

  players[0] = { team: 0, shirt: 14, x: 10, y: -4 };
  let items = tracker.update(players, 0);
  assert.equal(items.length, 1);
  const fx = items[0];
  assert.equal(fx.slot, 0);
  assert.equal(fx.team, 0);
  assert.equal(fx.offShirt, 6);
  assert.equal(fx.onShirt, 14);
  assert.equal(fx.progress, 0);
  assert.deepEqual(fx.outgoing, { x: 10, y: -4, shirt: 6, team: 0, alpha: 1 });
  assert.ok(close(fx.incoming.x, PAVILION[0].x + ((0 % 5) - 2) * 1.1));
  assert.ok(close(fx.incoming.y, PAVILION[0].y));
  assert.ok(fx.incoming.alpha > 0.3);
  assert.deepEqual([...tracker.hiddenSlots()], [0]);

  // The incoming ghost is the body the viewer should see for now.
  items = tracker.update(players, 0.2);
  assert.ok(items[0].progress > 0 && items[0].progress < 1);
  assert.ok(items[0].incoming.x > PAVILION[0].x - 3);
});

test('both ghosts reach their endpoints and release the live body', () => {
  const tracker = createSubstitutionTracker();
  const players = squad();
  tracker.update(players, 0);
  players[0] = { team: 0, shirt: 14, x: 10, y: -4 };

  let fx = tracker.update(players, 0)[0];
  const duration = fx.duration;
  assert.ok(duration >= 1.4 && duration <= 4.4);

  const towardPavilion = Math.hypot(
    fx.outgoing.x - fx.pavilion.x,
    fx.outgoing.y - fx.pavilion.y,
  );
  fx = tracker.update(players, duration / 2)[0];
  const closer = Math.hypot(
    fx.outgoing.x - fx.pavilion.x,
    fx.outgoing.y - fx.pavilion.y,
  );
  assert.ok(closer < towardPavilion, 'old shirt did not walk toward the bench');
  assert.ok(fx.incoming.alpha > 0.9);

  assert.deepEqual(tracker.update(players, duration), []);
  assert.equal(tracker.activeCount, 0);
  assert.equal(tracker.hiddenSlots().size, 0);
});

test('the walk-on tracks the live slot position', () => {
  const tracker = createSubstitutionTracker();
  const players = squad();
  tracker.update(players, 0);
  players[0] = { team: 0, shirt: 14, x: 10, y: -4 };
  tracker.update(players, 0);

  players[0].x = 22;
  players[0].y = 8;
  const fx = tracker.update(players, 0.5)[0];
  assert.deepEqual(fx.target, { x: 22, y: 8 });

  // Keep the body still so the remaining walk has a stable target.
  const rest = fx.duration - 0.5;
  const done = tracker.update(players, rest);
  assert.deepEqual(done, []);
});

test('a second swap in the same slot replaces the choreography', () => {
  const tracker = createSubstitutionTracker();
  const players = squad();
  tracker.update(players, 0);
  players[0] = { team: 0, shirt: 14, x: 10, y: -4 };
  tracker.update(players, 0);

  players[0] = { team: 0, shirt: 17, x: 12, y: -1 };
  const items = tracker.update(players, 0.1);
  assert.equal(items.length, 1);
  assert.equal(items[0].offShirt, 14);
  assert.equal(items[0].onShirt, 17);
});

test('substitutions on both teams animate independently', () => {
  const tracker = createSubstitutionTracker();
  const players = squad();
  tracker.update(players, 0);
  players[0] = { team: 0, shirt: 14, x: 10, y: -4 };
  players[2] = { team: 1, shirt: 15, x: -18, y: 2 };
  const items = tracker.update(players, 0);
  assert.equal(items.length, 2);
  assert.deepEqual(
    items.map((x) => [x.team, x.offShirt, x.onShirt]),
    [
      [0, 6, 14],
      [1, 4, 15],
    ],
  );
  assert.deepEqual([...tracker.hiddenSlots()].sort(), [0, 2]);
});

test('reset forgets the baseline and any active ghosts', () => {
  const tracker = createSubstitutionTracker();
  const players = squad();
  tracker.update(players, 0);
  players[0] = { team: 0, shirt: 14, x: 10, y: -4 };
  tracker.update(players, 0);
  assert.equal(tracker.activeCount, 1);

  tracker.reset();
  assert.equal(tracker.activeCount, 0);
  assert.deepEqual(tracker.update(players, 0), []);
});
