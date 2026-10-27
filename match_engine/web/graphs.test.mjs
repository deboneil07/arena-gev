import test from 'node:test';
import assert from 'node:assert/strict';

import { createMomentumTracker, createPassingNetwork, seqNumber } from './graphs.js';

test('seqNumber normalizes the wasm u64 BigInt event cursor', () => {
  assert.equal(seqNumber(42n), 42);
  assert.equal(typeof seqNumber(42n), 'number');
  assert.equal(seqNumber(0n), 0);
  // Number cursors pass through unchanged (older wasm builds).
  assert.equal(seqNumber(7), 7);
  assert.equal(seqNumber(0), 0);
});

test('seqNumber maps unknown cursors to the -1 sentinel', () => {
  assert.equal(seqNumber(undefined), -1);
  assert.equal(seqNumber(null), -1);
});

test('momentum dedupes on the exact string observation key', () => {
  const m = createMomentumTracker();
  const row = (t, dv, team = 0) => [{ t, dv, team }];
  m.observe('3:0', row(10, 0.5));
  m.observe('3:0', row(10, 0.5));
  assert.equal(m.home.length, 1);
  m.observe('3:1', row(10, 0.5));
  assert.equal(m.home.length, 2);
  // BigInt-derived seq plus row index can never collide with the -1 seed.
  assert.equal(m.home.reduce((sum, r) => sum + r.v, 0), 1.5);
});

test('passing network snapshots filter to a single team on request', () => {
  const net = createPassingNetwork();
  const p = (x, y, team, shirt) => ({ x, y, team, shirt });
  net.observePass(p(0, 0, 0, 9), p(10, 0, 0, 8));
  net.observePass(p(0, 0, 0, 9), p(10, 0, 0, 8));
  net.observePass(p(0, 0, 1, 5), p(-10, 0, 1, 4));

  const home = net.snapshot(8, 0);
  assert.equal(home.nodes.length, 2);
  assert.ok(home.nodes.every((n) => n.team === 0));
  assert.equal(home.links.length, 1);
  assert.equal(home.links[0].n, 2);
  assert.equal(home.maxTouches, 2);
  assert.equal(home.maxLink, 2);

  // Default (no team) keeps the old both-teams behaviour.
  const all = net.snapshot(8);
  assert.equal(all.nodes.length, 4);
  assert.equal(all.links.length, 2);
});

test('breakRetention ends the retained sequence but keeps accumulated stats', () => {
  const net = createPassingNetwork();
  const p = (x, y, shirt) => ({ x, y, team: 0, shirt });
  for (let i = 0; i < 3; i++) net.observePass(p(0, 0, 9), p(10, 0, 8));
  assert.equal(net.retention, 3);
  const before = net.snapshot(8, 0);
  assert.equal(before.nodes.length, 2);
  assert.equal(before.streak, 3);

  net.breakRetention();
  assert.equal(net.retention, 0);
  const after = net.snapshot(8, 0);
  assert.equal(after.streak, 0);
  assert.equal(after.nodes.length, before.nodes.length);
  assert.equal(after.links[0].n, before.links[0].n);

  // A new pass sequence starts from one.
  net.observePass(p(0, 0, 9), p(10, 0, 8));
  assert.equal(net.retention, 1);
});
