import test from 'node:test';
import assert from 'node:assert/strict';

import { createAttackLaneTracker } from './attackLanes.js';

function laneState() {
  return {
    active: true,
    projected: { x: 18.4, y: 7.9 },
    lanes: [
      { index: 0, y0: -34, y1: -17, centerY: -25.5, x0: 0, x1: 52.5, score: 0.2, momentum: 0, option: 0.2, projected: false, carrierLane: false },
      { index: 1, y0: -17, y1: 0, centerY: -8.5, x0: 0, x1: 52.5, score: 0.3, momentum: 0, option: 0.3, projected: false, carrierLane: false },
      { index: 2, y0: 0, y1: 17, centerY: 8.5, x0: 0, x1: 52.5, score: 0.9, momentum: 1, option: 0.4, projected: true, carrierLane: true },
      { index: 3, y0: 17, y1: 34, centerY: 25.5, x0: 0, x1: 52.5, score: 0.15, momentum: 0, option: 0.15, projected: false, carrierLane: false },
    ],
  };
}

test('attack lanes grow in and ease their scores', () => {
  const tracker = createAttackLaneTracker();
  const first = tracker.updateLive(0.1, laneState());
  assert.equal(first.length, 4);
  assert.ok(first.every((l) => l.alpha > 0 && l.alpha < 1));
  assert.ok(first.every((l) => l.growth > 0 && l.growth < 1));
  assert.equal(first[2].projected, true);
  assert.deepEqual(first[2].projectedPoint, { x: 18.4, y: 7.9 });
  assert.equal(first[0].projectedPoint, null);

  const grown = tracker.updateLive(0.6, laneState());
  assert.equal(grown.length, 4);
  assert.ok(grown.every((l) => l.alpha === 1));
  assert.ok(grown.every((l) => l.growth === 1));
  assert.ok(grown[2].score > grown[0].score);
  assert.ok(grown[2].score > 0.7);
  assert.equal(tracker.activeCount, 4);
});

test('attack lanes blur-fade when the carrier leaves the attacking half', () => {
  const tracker = createAttackLaneTracker();
  tracker.updateLive(0.6, laneState());
  const fading = tracker.updateLive(2.5, { active: false, lanes: [] });
  assert.equal(fading.length, 4);
  assert.ok(fading.every((l) => l.alpha < 1));
  assert.ok(fading.every((l) => l.blur > 0));
  tracker.updateLive(0.7, { active: false, lanes: [] });
  assert.equal(tracker.updateLive(0.1, { active: false, lanes: [] }).length, 0);
  assert.equal(tracker.activeCount, 0);
});

test('pass pulses travel down their destination lane and expire', () => {
  const tracker = createAttackLaneTracker();
  tracker.updateLive(0.6, laneState());
  tracker.pulse(2, 10, 30);
  const first = tracker.updateLive(0.1, laneState());
  const lane = first.find((l) => l.index === 2);
  assert.equal(lane.pulses.length, 1);
  assert.ok(lane.pulses[0].progress > 0 && lane.pulses[0].progress < 1);
  assert.equal(lane.pulses[0].fromX, 10);
  assert.equal(lane.pulses[0].toX, 30);
  assert.equal(tracker.pulseCount, 1);

  tracker.updateLive(0.7, laneState());
  assert.equal(tracker.pulseCount, 0);
  assert.equal(first.find((l) => l.index === 0).pulses.length, 0);
});

test('attack lane tracker resets', () => {
  const tracker = createAttackLaneTracker();
  tracker.updateLive(0.2, laneState());
  tracker.pulse(1, 0, 20);
  assert.equal(tracker.activeCount, 4);
  assert.equal(tracker.pulseCount, 1);
  tracker.reset();
  assert.equal(tracker.activeCount, 0);
  assert.equal(tracker.pulseCount, 0);
});

test('lane targets travel with the lane and survive the hold window', () => {
  const tracker = createAttackLaneTracker();
  const state = laneState();
  state.lanes[0] = {
    ...state.lanes[0],
    target: { x: 24, y: -22, shirt: 9, team: 0, p: 0.8 },
  };
  const first = tracker.updateLive(0.6, state);
  assert.equal(first.find((l) => l.index === 0).target.shirt, 9);
  const held = tracker.updateLive(1.0, { active: false, lanes: [] });
  assert.equal(held.find((l) => l.index === 0).target.shirt, 9);
});

test('lane memory can be tuned at runtime', () => {
  const tracker = createAttackLaneTracker();
  assert.equal(tracker.hold, 2.4);
  tracker.setHold(4.5);
  assert.equal(tracker.hold, 4.5);
  tracker.updateLive(0.6, laneState());
  tracker.updateLive(3.0, { active: false, lanes: [] });
  const stillHeld = tracker.updateLive(1.0, { active: false, lanes: [] });
  assert.equal(stillHeld.length, 4);
  assert.ok(stillHeld.every((l) => l.alpha === 1));
});

test('lane output carries its team so old-team ghosts can be dimmed', () => {
  const tracker = createAttackLaneTracker();
  const state = laneState();
  state.team = 1;
  const lanes = tracker.updateLive(0.6, state);
  assert.equal(lanes.length, 4);
  assert.ok(lanes.every((l) => l.team === 1));
});
