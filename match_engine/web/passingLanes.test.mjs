import test from 'node:test';
import assert from 'node:assert/strict';

import { createPassingLaneTracker } from './passingLanes.js';
import { createAttackChannelTracker } from './attackChannels.js';

const lane = {
  from: { x: 0, y: 0 },
  to: { x: 12, y: 4 },
  p: 0.72,
  player: { team: 0, shirt: 9 },
};

test('passing lanes grow toward the live receiver and follow its movement', () => {
  const tracker = createPassingLaneTracker();
  let live = { x: 12, y: 4 };
  const resolve = () => ({ from: { x: 0, y: 0 }, to: live });

  const first = tracker.updateLive(0.1, [lane], resolve);
  assert.equal(first.length, 1);
  assert.ok(first[0].alpha > 0 && first[0].alpha < 1);
  assert.ok(first[0].growth > 0 && first[0].growth < 1);
  assert.deepEqual(first[0].from, { x: 0, y: 0 });

  const grown = tracker.updateLive(0.4, [lane], resolve);
  assert.equal(grown.length, 1);
  assert.equal(grown[0].growth, 1);
  assert.equal(grown[0].alpha, 1);

  // The receiver keeps running: the arrow endpoint eases toward the new spot.
  live = { x: 20, y: 8 };
  const moved = tracker.updateLive(0.5, [lane], resolve);
  assert.ok(moved[0].to.x > 12, 'endpoint follows the live receiver');
  assert.equal(moved[0].key, '0:9');
});

test('passing lanes fade out and blur when the option disappears', () => {
  const tracker = createPassingLaneTracker();
  const resolve = () => ({ from: { x: 0, y: 0 }, to: { x: 12, y: 4 } });
  tracker.updateLive(0.4, [lane], resolve);
  const fading = tracker.updateLive(0.2, [], resolve);
  assert.equal(fading.length, 1);
  assert.ok(fading[0].alpha < 1);
  assert.ok(fading[0].blur > 0);
  tracker.updateLive(0.6, [], resolve);
  assert.equal(tracker.updateLive(0.1, [], resolve).length, 0);
  assert.equal(tracker.activeCount, 0);
});

test('passing lane tracker resets', () => {
  const tracker = createPassingLaneTracker();
  tracker.updateLive(0.2, [lane], () => null);
  assert.equal(tracker.activeCount, 1);
  tracker.reset();
  assert.equal(tracker.activeCount, 0);
});

test('attack channels follow live runners and keep their curve/score', () => {
  const tracker = createAttackChannelTracker();
  let live = { x: 20, y: 0 };
  const channel = {
    from: { x: 20, y: 0 },
    to: { x: 32, y: -4 },
    player: { team: 0, shirt: 9 },
    score: 0.8,
    curve: 2.5,
  };
  const resolve = () => live;

  const first = tracker.updateLive(0.1, [channel], resolve);
  assert.equal(first.length, 1);
  assert.ok(first[0].growth > 0 && first[0].growth < 1);
  assert.equal(first[0].curve, 2.5);
  assert.equal(first[0].score, 0.8);

  live = { x: 24, y: 2 };
  const moved = tracker.updateLive(0.4, [channel], resolve);
  assert.deepEqual(moved[0].from, { x: 24, y: 2 });
  assert.equal(moved[0].growth, 1);

  const fading = tracker.updateLive(0.2, [], resolve);
  assert.ok(fading[0].alpha < 1);
  tracker.updateLive(0.6, [], resolve);
  assert.equal(tracker.updateLive(0.1, [], resolve).length, 0);
});
