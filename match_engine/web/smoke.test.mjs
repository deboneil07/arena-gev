import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

import { EFFECTS } from './effects.js';

// ── Static wiring ──────────────────────────────────────────────
// The app reaches the page by id and by data-effect; both contracts are
// checked against the real index.html before any runtime smoke runs.

const here = path.dirname(fileURLToPath(import.meta.url));
const html = await readFile(path.join(here, 'index.html'), 'utf8');
const appSource = await readFile(path.join(here, 'app.js'), 'utf8');

test('every element app.js looks up exists in index.html', () => {
  const ids = [...appSource.matchAll(/getElementById\('([\w-]+)'\)/g)].map((m) => m[1]);
  assert.ok(ids.length > 30, `expected many lookups, found ${ids.length}`);
  const missing = ids.filter((id) => !html.includes(`id="${id}"`));
  assert.deepEqual(missing, [], `missing ids: ${missing.join(', ')}`);
});

test('the effects bar, the panel shell and the view controls are all present', () => {
  const buttons = [...html.matchAll(/data-effect="([\w-]+)"/g)].map((m) => m[1]);
  assert.deepEqual([...buttons].sort(), EFFECTS.map((e) => e.key).sort());
  for (const id of ['player-panel', 'player-context', 'stage', 'zoom', 'zoomval',
    'reset-view', 'zone-alpha', 'zone-alphaval', 'gev-canvas']) {
    assert.ok(html.includes(`id="${id}"`), `index.html lacks #${id}`);
  }
  assert.match(html, /role="tablist"/, 'panel tabs exist in the markup');
  assert.match(html, /app\.js\?v=\d+/, 'the module is cache-busted');
});

// ── Runtime smoke ──────────────────────────────────────────────
// A stub DOM good enough to boot app.js: module top level runs, the
// animation loop paints a few frames, and any exception (including an
// import that names an export which does not exist) fails the test.

function camelizeData(name) {
  return name.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase());
}

function makeRecordingContext(canvas) {
  const calls = [];
  const ctx = new Proxy({}, {
    get(target, prop) {
      if (prop === 'canvas') return canvas;
      if (prop === 'calls') return calls;
      if (prop === 'measureText') return () => ({ width: 42 });
      if (prop === 'createRadialGradient' || prop === 'createLinearGradient') {
        return () => ({ addColorStop() {} });
      }
      if (typeof prop === 'string') return (...args) => { calls.push([prop, ...args]); };
      return target[prop];
    },
    set(target, prop, value) { target[prop] = value; return true; },
  });
  ctx.calls = calls;
  return ctx;
}

class StubElement {
  constructor(tag = 'div') {
    this.nodeType = 1;
    this.tagName = String(tag).toUpperCase();
    this.children = [];
    this.attrs = Object.create(null);
    this.listeners = Object.create(null);
    this.dataset = {};
    this.style = {};
    this.className = '';
    this.tabIndex = -1;
    this.value = '';
    this.hidden = false;
    this._text = '';
    this._html = '';
    this.width = this.tagName === 'CANVAS' ? 1050 : 0;
    this.height = this.tagName === 'CANVAS' ? 680 : 0;
    this._ctx = null;
  }

  get classList() {
    const node = this;
    const read = () => new Set(node.className.split(/\s+/).filter(Boolean));
    const write = (s) => { node.className = [...s].join(' '); };
    return {
      contains: (c) => read().has(c),
      add: (c) => { const s = read(); s.add(c); write(s); },
      remove: (c) => { const s = read(); s.delete(c); write(s); },
      toggle: (c, force) => {
        const s = read();
        const next = force === undefined ? !s.has(c) : Boolean(force);
        if (next) s.add(c); else s.delete(c);
        write(s);
        return next;
      },
    };
  }

  setAttribute(name, value) {
    this.attrs[name] = String(value);
    if (name === 'class') this.className = String(value);
    if (name.startsWith('data-')) this.dataset[camelizeData(name)] = String(value);
  }

  getAttribute(name) { return name in this.attrs ? this.attrs[name] : null; }
  removeAttribute(name) { delete this.attrs[name]; }

  get textContent() { return this._text; }
  set textContent(v) { this._text = String(v); this.children = []; }
  get innerHTML() { return this._html; }
  set innerHTML(v) { this._html = String(v); this.children = []; }

  append(...nodes) { this.children.push(...nodes.filter(Boolean)); }
  appendChild(node) { this.append(node); return node; }
  remove() {}
  contains() { return false; }
  focus() {}
  click() { for (const fn of this.listeners.click ?? []) fn({ type: 'click', target: this }); }

  addEventListener(type, fn) { (this.listeners[type] ??= []).push(fn); }
  removeEventListener() {}

  querySelector() { return new StubElement(); }
  querySelectorAll() { return []; }

  getBoundingClientRect() {
    return { left: 0, top: 0, right: this.width, bottom: this.height,
      width: this.width, height: this.height };
  }

  getContext() {
    if (!this._ctx) this._ctx = makeRecordingContext(this);
    return this._ctx;
  }
}

// Effect buttons parsed from the real markup so the smoke run exercises
// the same controls a browser would.
const effectButtons = [...html.matchAll(/<button[^>]*data-effect="([\w-]+)"[^>]*>/g)].map((m) => {
  const button = new StubElement('button');
  button.setAttribute('data-effect', m[1]);
  button.setAttribute('aria-pressed', 'true');
  return button;
});
assert.equal(effectButtons.length, EFFECTS.length, 'parsed every effect button');

const elements = new Map();
function elementFor(id) {
  if (!elements.has(id)) {
    const el = new StubElement(id === 'gev-canvas' ? 'canvas' : 'div');
    el.id = id;
    elements.set(id, el);
  }
  return elements.get(id);
}

const frameQueue = [];
let frameTime = 0;

globalThis.document = {
  getElementById: elementFor,
  createElement: (tag) => new StubElement(tag),
  createTextNode: (data) => ({ nodeType: 3, data: String(data) }),
  querySelectorAll: (sel) => (sel === '[data-effect]' ? effectButtons : []),
  querySelector: () => null,
  addEventListener() {},
  removeEventListener() {},
  body: new StubElement('body'),
  currentScript: null,
  activeElement: null,
};
globalThis.window = {};
globalThis.location = { search: '' };
globalThis.requestAnimationFrame = (fn) => { frameQueue.push(fn); return frameQueue.length; };
globalThis.fetch = async () => { throw new Error('offline smoke test'); };

function runFrames(count, stepMs = 16) {
  for (let i = 0; i < count; i++) {
    const queued = frameQueue.splice(0, frameQueue.length);
    frameTime += stepMs;
    for (const fn of queued) fn(frameTime);
  }
}

test('app.js boots and paints frames under a stub DOM', async () => {
  // A bad import name would throw a SyntaxError here — that is the point.
  await import('./app.js');

  // Let main() settle its rejected init/fetch promises first.
  await new Promise((resolve) => setTimeout(resolve, 0));
  runFrames(3);

  const status = elementFor('status').textContent;
  assert.ok(!status.startsWith('Frame error'), `render loop threw: ${status}`);

  const canvas = elementFor('gev-canvas');
  const calls = canvas.getContext('2d').calls;
  const names = new Set(calls.map(([name]) => name));
  assert.ok(names.has('fillRect'), 'the view-transform base fill ran');
  assert.ok(names.has('setTransform'), 'the view transform is applied every frame');
  assert.ok(names.has('strokeRect'), 'the pitch was drawn');

  // Effect buttons were wired: every click handler registered.
  for (const button of effectButtons) {
    assert.ok((button.listeners.click ?? []).length >= 1,
      `no click handler for ${button.getAttribute('data-effect')}`);
  }

  // The panel shell was constructed against the stub and stays inert
  // until a match (and its analysis log) arrives.
  assert.ok(elementFor('player-panel'), 'panel shell resolved');
});
