import test from 'node:test';
import assert from 'node:assert/strict';

import { createShapeTracker } from './shape.js';

const line = (x, y0 = -10, y1 = 10) => [{ x, y: y1 }, { x, y: y0 }];
const units = (defX = -30, midX = -10, fwdX = 20) => ({
  def: line(defX),
  mid: line(midX),
  fwd: line(fwdX),
});

test('the tracker eases a jumped line instead of snapping', () => {
  const tracker = createShapeTracker({ tau: 0.5, hold: 0.9, fade: 0.3 });
  for (let i = 0; i < 10; i++) tracker.update(0, units(0), 0.1);
  const before = tracker.update(0, units(0), 0.1);
  assert.ok(Math.abs(before.units.def.points[0].x) < 1e-6);
  const after = tracker.update(0, units(20), 0.1);
  assert.ok(after.units.def.points[0].x > 0);
  assert.ok(after.units.def.points[0].x < 20, 'must lag the live target');
  for (let i = 0; i < 100; i++) tracker.update(0, units(20), 0.1);
  const settled = tracker.update(0, units(20), 0.1);
  assert.ok(Math.abs(settled.units.def.points[0].x - 20) < 0.1);
});

test('a unit that empties is held, then fades out', () => {
  const tracker = createShapeTracker({ tau: 0.2, hold: 0.5, fade: 0.2 });
  for (let i = 0; i < 10; i++) tracker.update(0, units(), 0.1);
  let shape = tracker.update(0, { def: [], mid: units().mid, fwd: units().fwd }, 0.1);
  assert.equal(shape.units.def.points.length, 2, 'line is held');
  for (let i = 0; i < 6; i++) {
    shape = tracker.update(0, { def: [], mid: units().mid, fwd: units().fwd }, 0.1);
  }
  assert.ok(shape.units.def.alpha < 1, 'fade has started');
  for (let i = 0; i < 20; i++) {
    shape = tracker.update(0, { def: [], mid: units().mid, fwd: units().fwd }, 0.1);
  }
  assert.equal(shape.units.def.points.length, 0);
  assert.equal(shape.units.def.alpha, 0);
});

test('units fade in and the triangle only appears when all three exist', () => {
  const tracker = createShapeTracker({ tau: 0.2, hold: 0.4, fade: 0.2 });
  let shape = tracker.update(0, units(), 0.05);
  assert.ok(shape.units.def.alpha > 0 && shape.units.def.alpha < 1);
  assert.ok(!shape.triangle || shape.triangleAlpha < 1);
  for (let i = 0; i < 10; i++) shape = tracker.update(0, units(), 0.1);
  assert.ok(shape.triangle);
  assert.ok(shape.triangleAlpha > 0.9);

  for (let i = 0; i < 3; i++) {
    shape = tracker.update(0, { def: [], mid: units().mid, fwd: units().fwd }, 0.1);
  }
  assert.ok(shape.triangle, 'triangle held briefly');
});

test('membership changes hold the previous line before adopting the new one', () => {
  const tracker = createShapeTracker({ tau: 0.2, hold: 0.4, fade: 0.2 });
  for (let i = 0; i < 10; i++) tracker.update(0, units(), 0.1);
  let shape = tracker.update(0, {
    def: [{ x: -30, y: 10 }, { x: -30, y: 0 }, { x: -30, y: -10 }],
    mid: units().mid,
    fwd: units().fwd,
  }, 0.1);
  assert.equal(shape.units.def.points.length, 2, 'old membership held');
  for (let i = 0; i < 5; i++) {
    shape = tracker.update(0, {
      def: [{ x: -30, y: 10 }, { x: -30, y: 0 }, { x: -30, y: -10 }],
      mid: units().mid,
      fwd: units().fwd,
    }, 0.1);
  }
  assert.equal(shape.units.def.points.length, 3);
});

test('reset clears both teams', () => {
  const tracker = createShapeTracker();
  for (let i = 0; i < 10; i++) tracker.update(0, units(), 0.1);
  tracker.reset();
  const shape = tracker.update(0, { def: [], mid: [], fwd: [] }, 0.1);
  assert.equal(shape.units.def.points.length, 0);
  assert.equal(shape.triangle, null);
});
