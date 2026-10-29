// Match graphs: momentum/xT timeline, passing network, shot map (all Canvas,
// all renderer-side, all fed by engine getters + analysis maths).
//
// Nothing here touches the sim: graphs accumulate from the same snapshots
// the overlays use, so halftime/fulltime views are free at any seek point.

import { shotXg, xtAt, xtGain } from './analysis.js';

/**
 * Normalize the engine's u64 event cursor. wasm-bindgen exposes u64 as a JS
 * BigInt, and mixing BigInt with Number throws; unknown cursors map to -1.
 */
export function seqNumber(v) {
  return typeof v === 'bigint' ? Number(v) : (v ?? -1);
}

/**
 * Cumulative threat timeline. Feed rows drive it: completed passes add
 * xT gain, shots add xG, goals/cards mark the line. One series per team.
 */
export function createMomentumTracker() {
  let home = [];
  let away = [];
  let lastKey = -1;

  function observe(key, rows) {
    if (key === lastKey) return;
    lastKey = key;
    for (const r of rows) {
      const series = r.team === 0 ? home : r.team === 1 ? away : null;
      if (!series) continue;
      const prev = series.length ? series[series.length - 1].v : 0;
      series.push({ t: r.t, v: prev + (r.dv || 0), label: r.label, type: r.type });
    }
  }

  function reset() {
    home = [];
    away = [];
    lastKey = -1;
  }

  return { observe, reset, get home() { return home; }, get away() { return away; } };
}

/** Value of a feed row for the momentum strip (model estimates). */
export function rowValue(type, label) {
  if (type === 16) return { dv: 1.0, big: true };
  if (type === 15 || type === 13 || type === 14) return { dv: 0.12, big: true };
  if (type === 17) return { dv: 0.0, big: true };
  if (type === 1 || type === 2) return { dv: 0.015, big: false };
  return { dv: 0.0, big: false };
}

/**
 * Passing network: average positions + weighted links from completed passes.
 *
 * Two modes share one accumulator:
 * - `mesh` (match-long): every roster shirt is a node and every pair is a
 *   link, with both directions summed. This is the complete team web.
 * - `outlet` (possession-window): each player keeps exactly one wire — his
 *   most-used teammate in the current run of possession. The window clears on
 *   a turnover, so the shape always describes the attack being watched.
 */
// Link weight halves roughly every two minutes without a pass, so a pair
// that stopped combining thins back into the quiet mesh instead of staying a
// thick wire for the rest of the match.
export const NETWORK_DECAY_TAU = 180;
const WINDOW_LINK_DECAY_TAU = 90;

export function createPassingNetwork() {
  const touches = new Map(); // key team:shirt -> {x,y,n}
  const links = new Map(); // key from>to -> n
  const linkLast = new Map(); // key from>to -> last pass sim time
  const windowTouches = new Map();
  const windowLinks = new Map();
  const windowLinkLast = new Map();
  let lastTeam = null;
  let streak = 0;
  // Match-long memory is tunable from the UI; the outlet window is kept
  // shorter so a possession's shape stays responsive.
  let meshTau = NETWORK_DECAY_TAU;

  function addTouch(map, p) {
    const k = `${p.team}:${p.shirt}`;
    const t = map.get(k) ?? { x: 0, y: 0, n: 0, team: p.team, shirt: p.shirt };
    t.x += p.x;
    t.y += p.y;
    t.n += 1;
    map.set(k, t);
  }

  function addLink(map, key, time) {
    map.set(key, (map.get(key) ?? 0) + 1);
    return time;
  }

  function clearWindow() {
    windowTouches.clear();
    windowLinks.clear();
    windowLinkLast.clear();
  }

  function observePass(from, to, time = null) {
    if (!from || !to) return;
    const at = Number.isFinite(time) ? time : null;
    // Both ends of a pass are touches: the passer and the receiver both need
    // a node, otherwise links to a player who has not passed yet disappear.
    addTouch(touches, from);
    addTouch(touches, to);
    const lk = `${from.team}:${from.shirt}>${to.team}:${to.shirt}`;
    addLink(links, lk, at);
    if (at !== null) linkLast.set(lk, at);
    // Retention: consecutive passes by the same team build the streak.
    if (from.team === to.team) {
      if (lastTeam === from.team && streak > 0) {
        streak += 1;
      } else {
        // New possession sequence: the outlet window starts here.
        clearWindow();
        lastTeam = from.team;
        streak = 1;
      }
      addTouch(windowTouches, from);
      addTouch(windowTouches, to);
      addLink(windowLinks, lk, at);
      if (at !== null) windowLinkLast.set(lk, at);
    } else {
      lastTeam = null;
      streak = 0;
      clearWindow();
    }
  }

  /**
   * Snapshot of the network.
   *
   * `roster` (optional) is the followed team's live squad: when supplied,
   * every listed shirt becomes a node (even with zero touches).
   *
   * `mode` selects the link rule:
   * - `'mesh'` (default) keeps the complete pair web and its match-long stats;
   * - `'outlet'` keeps one wire per player to his top teammate in the current
   *   possession window, so the layer reads as this attack's shape.
   */
  function snapshot(topLinks = 5, team = null, roster = null, mode = 'mesh', time = null) {
    const teamFilter = team === 0 || team === 1 ? team : null;
    const outlet = mode === 'outlet';
    const sourceTouches = outlet ? windowTouches : touches;
    const sourceLinks = outlet ? windowLinks : links;
    const sourceLast = outlet ? windowLinkLast : linkLast;
    const tau = outlet ? Math.max(20, meshTau * 0.5) : meshTau;
    const at = Number.isFinite(time) ? time : null;
    const weightOf = (key) => {
      const raw = sourceLinks.get(key) ?? 0;
      if (at === null || raw <= 0) return raw;
      const last = sourceLast.get(key);
      if (!Number.isFinite(last)) return raw;
      return raw * Math.exp(-Math.max(0, at - last) / tau);
    };
    const nodes = [];
    const nodeByKey = new Map();

    function addNode(key, fallback) {
      if (nodeByKey.has(key)) return;
      const t = sourceTouches.get(key);
      const node = t
        ? {
            x: t.x / Math.max(1, t.n),
            y: t.y / Math.max(1, t.n),
            n: t.n,
            team: t.team,
            shirt: t.shirt,
          }
        : fallback;
      nodes.push(node);
      nodeByKey.set(key, node);
    }

    for (const r of roster ?? []) {
      if (!r) continue;
      if (teamFilter !== null && r.team !== teamFilter) continue;
      const key = `${r.team}:${r.shirt}`;
      addNode(key, {
        x: Number.isFinite(r.x) ? r.x : 0,
        y: Number.isFinite(r.y) ? r.y : 0,
        n: 0,
        team: r.team,
        shirt: r.shirt,
      });
    }
    for (const [key, t] of sourceTouches) {
      if (teamFilter !== null && t.team !== teamFilter) continue;
      addNode(key, {
        x: t.x / Math.max(1, t.n),
        y: t.y / Math.max(1, t.n),
        n: t.n,
        team: t.team,
        shirt: t.shirt,
      });
    }

    let out;
    if (outlet) {
      // One outgoing wire per player: his most-used teammate in this window.
      const bestOut = new Map();
      for (const key of sourceLinks.keys()) {
        const n = weightOf(key);
        if (n <= 0) continue;
        const [a, b] = key.split('>');
        if (!nodeByKey.has(a) || !nodeByKey.has(b)) continue;
        const cur = bestOut.get(a);
        if (!cur || n > cur.n) bestOut.set(a, { b, n });
      }
      // If two players choose each other, draw one wire rather than two.
      const pairs = new Map();
      for (const [a, { b, n }] of bestOut) {
        const pair = a < b ? `${a}|${b}` : `${b}|${a}`;
        const cur = pairs.get(pair);
        if (!cur || n > cur.n) pairs.set(pair, { a, b, n });
      }
      out = [...pairs.values()].sort((x, y) => x.n - y.n);
      if (!roster || roster.length === 0) {
        out = out.slice(-Math.max(0, topLinks));
      }
    } else if (roster && roster.length > 0) {
      // Complete mesh: one undirected link per pair, both directions summed.
      const seen = new Set();
      out = [];
      for (const a of nodes) {
        for (const b of nodes) {
          if (a === b) continue;
          const ka = `${a.team}:${a.shirt}`;
          const kb = `${b.team}:${b.shirt}`;
          const pair = ka < kb ? `${ka}|${kb}` : `${kb}|${ka}`;
          if (seen.has(pair)) continue;
          seen.add(pair);
          out.push({
            a: ka,
            b: kb,
            n: weightOf(`${ka}>${kb}`) + weightOf(`${kb}>${ka}`),
          });
        }
      }
      out.sort((x, y) => x.n - y.n);
    } else {
      const teamOf = (key) => Number(key.split(':')[0]);
      out = [...sourceLinks.keys()]
        .map((k) => ({ k, n: weightOf(k) }))
        .filter(({ k, n }) => {
          if (n <= 0) return false;
          if (teamFilter === null) return true;
          const [a, b] = k.split('>');
          return teamOf(a) === teamFilter && teamOf(b) === teamFilter;
        })
        .sort((a, b) => b.n - a.n)
        .slice(0, topLinks)
        .map(({ k, n }) => {
          const [a, b] = k.split('>');
          return { a, b, n };
        });
    }
    return {
      nodes,
      links: out,
      maxTouches: nodes.reduce((m, n) => Math.max(m, n.n), 0),
      maxLink: out.reduce((m, l) => Math.max(m, l.n), 0),
      streak,
      mode: outlet ? 'outlet' : 'mesh',
    };
  }

  /** Interception / turnover: end the retained sequence and its outlet window. */
  function breakRetention() {
    lastTeam = null;
    streak = 0;
    clearWindow();
  }

  function reset() {
    touches.clear();
    links.clear();
    linkLast.clear();
    clearWindow();
    lastTeam = null;
    streak = 0;
  }

  function setDecay(seconds) {
    const next = Number(seconds);
    if (Number.isFinite(next) && next > 0) meshTau = Math.max(20, Math.min(900, next));
  }

  return {
    observePass,
    snapshot,
    setDecay,
    get decay() { return meshTau; },
    breakRetention,
    reset,
    get retention() { return streak; },
  };
}

export { shotXg, xtAt, xtGain };
