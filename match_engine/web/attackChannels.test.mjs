import test from 'node:test';
import assert from 'node:assert/strict';

import { createAttackChannelTracker } from './attackChannels.js';

test('attack channels follow live runners and keep their curve/score', () => {
  const tracker = createAttackChannelTracker();
  let live = { x: 20, y: 0 };
  const channel = {
    from: { x: 20, y: 0 },
    to: { x: 32, y: -4 },
    player: { team: 0, shirt: 9 },
    score: 0.8,
    curve: 2.5,
    landing: { lane: 1, y0: -17, y1: 0, centerY: -8.5, x0: 0, x1: 52.5, x: 32, y: -4 },
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
  assert.deepEqual(moved[0].landing, channel.landing);

  const fading = tracker.updateLive(0.2, [], resolve);
  assert.ok(fading[0].alpha < 1);
  tracker.updateLive(0.6, [], resolve);
  assert.equal(tracker.updateLive(0.1, [], resolve).length, 0);
});

test('attack channel tracker resets', () => {
  const tracker = createAttackChannelTracker();
  const channel = {
    from: { x: 20, y: 0 },
    to: { x: 32, y: -4 },
    player: { team: 0, shirt: 9 },
    score: 0.8,
    curve: 0,
  };
  tracker.updateLive(0.2, [channel], () => channel.from);
  assert.equal(tracker.activeCount, 1);
  tracker.reset();
  assert.equal(tracker.activeCount, 0);
});
