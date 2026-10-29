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

test('passing network mesh includes every roster shirt and every pair', () => {
  const net = createPassingNetwork();
  const p = (x, y, team, shirt) => ({ x, y, team, shirt });
  net.observePass(p(0, 0, 0, 9), p(10, 0, 0, 8));
  net.observePass(p(0, 0, 0, 9), p(10, 0, 0, 8));
  net.observePass(p(10, 0, 0, 8), p(0, 0, 0, 9)); // reverse direction sums

  const roster = [
    { team: 0, shirt: 9, x: 0, y: 0 },
    { team: 0, shirt: 8, x: 10, y: 0 },
    { team: 0, shirt: 1, x: -40, y: 0 }, // no touches yet
  ];
  const snap = net.snapshot(8, 0, roster);
  assert.equal(snap.nodes.length, 3);
  const keeper = snap.nodes.find((n) => n.shirt === 1);
  assert.equal(keeper.n, 0);
  assert.equal(keeper.x, -40);
  // Complete mesh for three players, weakest first.
  assert.equal(snap.links.length, 3);
  assert.equal(snap.links[0].n, 0);
  assert.equal(snap.links[snap.links.length - 1].n, 3);
  assert.equal(snap.maxLink, 3);
  // Mesh links are undirected: one entry per pair.
  const pairs = new Set(snap.links.map((l) => [l.a, l.b].sort().join('|')));
  assert.equal(pairs.size, snap.links.length);
});

test('outlet mode keeps one wire per player in the current possession', () => {
  const net = createPassingNetwork();
  const p = (x, y, shirt) => ({ x, y, team: 0, shirt });
  net.observePass(p(0, 0, 9), p(10, 0, 8));
  net.observePass(p(10, 0, 8), p(0, 0, 9));
  net.observePass(p(0, 0, 9), p(10, 0, 8));
  net.observePass(p(10, 0, 8), p(0, 0, 9));
  net.observePass(p(0, 0, 9), p(10, 0, 8));
  net.observePass(p(0, 0, 9), p(-10, 10, 11));
  net.observePass(p(-10, 10, 11), p(10, 0, 8));
  const roster = [
    { team: 0, shirt: 9, x: 0, y: 0 },
    { team: 0, shirt: 8, x: 10, y: 0 },
    { team: 0, shirt: 1, x: -40, y: 0 },
  ];
  const snap = net.snapshot(8, 0, roster, 'outlet');
  assert.equal(snap.mode, 'outlet');
  assert.equal(snap.streak, 7);
  // Roster 9/8/1 plus shirt 11 from the possession touches.
  assert.equal(snap.nodes.length, 4);
  assert.equal(snap.nodes.find((n) => n.shirt === 1).n, 0);
  // 9 -> 8 is each man's top outlet, so the pair is drawn once; 11 -> 8 is
  // the only other wire. Two wires, not the six-pair mesh.
  assert.equal(snap.links.length, 2);
  const pair = (a, b) => [a, b].sort().join('|');
  const nineEight = snap.links.find((l) => pair(l.a, l.b) === '0:8|0:9');
  assert.equal(nineEight.n, 3);
  const elevenEight = snap.links.find((l) => pair(l.a, l.b) === pair('0:8', '0:11'));
  assert.equal(elevenEight.n, 1);
  assert.equal(snap.maxLink, 3);

  // A turnover clears the possession window, not the match-long stats.
  net.breakRetention();
  const after = net.snapshot(8, 0, roster, 'outlet');
  assert.equal(after.streak, 0);
  assert.equal(after.links.length, 0);
  assert.ok(after.nodes.every((n) => n.n === 0));
  const mesh = net.snapshot(8, 0, roster, 'mesh');
  assert.equal(mesh.mode, 'mesh');
  assert.equal(mesh.links.length, 6);
  assert.equal(mesh.links.find((l) => pair(l.a, l.b) === '0:8|0:9').n, 5);
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

test('passing links decay when a pair stops combining', () => {
  const net = createPassingNetwork();
  const p = (x, y, shirt) => ({ x, y, team: 0, shirt });
  for (let i = 0; i < 10; i++) net.observePass(p(0, 0, 9), p(10, 0, 8), 0);
  const fresh = net.snapshot(8, 0, null, 'mesh', 0);
  const old = net.snapshot(8, 0, null, 'mesh', 360);
  const link = (snap) => snap.links.find((l) => (l.a === '0:9' && l.b === '0:8') || (l.a === '0:8' && l.b === '0:9'));
  assert.ok(fresh && old);
  assert.ok(link(old).n < link(fresh).n);
  // A fresh pass refreshes the timestamp and thickens it again.
  net.observePass(p(0, 0, 9), p(10, 0, 8), 360);
  const refreshed = net.snapshot(8, 0, null, 'mesh', 360);
  assert.ok(link(refreshed).n > link(old).n);
});

test('outlet window links also thin without recent passes', () => {
  const net = createPassingNetwork();
  const p = (x, y, shirt) => ({ x, y, team: 0, shirt });
  for (let i = 0; i < 4; i++) net.observePass(p(0, 0, 9), p(10, 0, 8), 0);
  const fresh = net.snapshot(8, 0, null, 'outlet', 0);
  const old = net.snapshot(8, 0, null, 'outlet', 300);
  assert.ok(fresh.links.length === 1 && old.links.length === 1);
  assert.ok(old.links[0].n < fresh.links[0].n);
});

test('network memory controls how fast old links thin', () => {
  const net = createPassingNetwork();
  const p = (x, y, shirt) => ({ x, y, team: 0, shirt });
  net.observePass(p(0, 0, 9), p(10, 0, 8), 0);
  net.setDecay(60);
  assert.equal(net.decay, 60);
  const slow = createPassingNetwork();
  slow.observePass(p(0, 0, 9), p(10, 0, 8), 0);
  slow.setDecay(600);
  const shortMemory = net.snapshot(8, 0, null, 'mesh', 120);
  const longMemory = slow.snapshot(8, 0, null, 'mesh', 120);
  const weight = (snap) => snap.links[0]?.n ?? 0;
  assert.ok(weight(longMemory) > weight(shortMemory));
});
