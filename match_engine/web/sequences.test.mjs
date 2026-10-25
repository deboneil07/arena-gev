import test from 'node:test';
import assert from 'node:assert/strict';

import { SEQUENCE, createCelebrationTracker, createFxTracker } from './sequences.js';

const p = (x, y, team = 0, shirt = 9) => ({ x, y, team, shirt });

test('goal celebration runs the scorer to the corner with a mob', () => {
  const t = createCelebrationTracker({ duration: 3.0, mobCount: 2 });
  const scorer = p(40, 5, 0, 9);
  const mates = [p(30, 0, 0, 7), p(25, -5, 0, 10), p(20, 0, 0, 8)];
  t.trigger(scorer, mates);
  assert.ok(t.active);
  const early = t.update([scorer, ...mates], { x: 0, y: 0 }, 0.5);
  assert.ok(early.run);
  assert.equal(early.mob.length, 2);
  // Scorer moves toward the corner arc, mates toward the scorer.
  assert.ok(early.run.x > scorer.x);
  // Expires on its own.
  let last = null;
  for (let i = 0; i < 200; i++) last = t.update([scorer, ...mates], { x: 0, y: 0 }, 0.05);
  assert.equal(last, null);
  assert.ok(!t.active);
});

test('celebration ignores a missing scorer', () => {
  const t = createCelebrationTracker();
  t.trigger(null, [p(0, 0)]);
  assert.ok(!t.active);
});

test('fx tracker spawns and expires save/post/flag effects', () => {
  const fx = createFxTracker();
  fx.spawn('save', 50, 0);
  fx.spawn('post', 52.5, 3.66);
  fx.spawn('flag', 10, 5);
  let items = fx.update(0.1);
  assert.equal(items.length, 3);
  assert.ok(items.every((e) => e.progress > 0 && e.progress < 1));
  // NaN positions never spawn.
  fx.spawn('save', NaN, 0);
  items = fx.update(0.1);
  assert.equal(items.length, 3);
  // Advance past every duration.
  for (let i = 0; i < 100; i++) items = fx.update(0.1);
  assert.equal(items.length, 0);
});

test('fx tracker resets on seek', () => {
  const fx = createFxTracker();
  fx.spawn('save', 50, 0);
  fx.reset();
  assert.equal(fx.update(0.1).length, 0);
});
