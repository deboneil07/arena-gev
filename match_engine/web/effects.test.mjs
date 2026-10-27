import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { EFFECTS, SHUFFLE_SECONDS, createEffectController } from './effects.js';

const keys = EFFECTS.map((effect) => effect.key);

test('the effects bar exposes every controller layer and the shuffle button', () => {
  const html = readFileSync(new URL('./index.html', import.meta.url), 'utf8');
  for (const key of keys) {
    assert.match(html, new RegExp(`data-effect="${key}"`));
  }
  assert.match(html, /id="shuffle-effects"/);
});

test('every analytical layer is enabled and visible by default', () => {
  const effects = createEffectController();
  assert.equal(effects.shuffleActive, false);
  assert.equal(effects.interval, SHUFFLE_SECONDS);
  for (const key of keys) {
    assert.equal(effects.isEnabled(key), true);
    assert.equal(effects.isVisible(key), true);
  }
});

test('individual toggles hide only their own layer', () => {
  const effects = createEffectController();
  assert.equal(effects.toggleEnabled('network'), true);
  assert.equal(effects.isEnabled('network'), false);
  assert.equal(effects.isVisible('network'), false);
  for (const key of keys.filter((k) => k !== 'network')) {
    assert.equal(effects.isVisible(key), true);
  }
  effects.toggleEnabled('network');
  assert.equal(effects.isVisible('network'), true);
});

test('shuffle shows one enabled layer and advances every four seconds', () => {
  const effects = createEffectController();
  effects.setShuffle(true);
  assert.equal(effects.shuffleActive, true);
  assert.equal(effects.currentKey, keys[0]);
  assert.equal(effects.isVisible(keys[0]), true);
  for (const key of keys.slice(1)) assert.equal(effects.isVisible(key), false);

  effects.update(SHUFFLE_SECONDS - 0.1);
  assert.equal(effects.currentKey, keys[0]);
  effects.update(0.1);
  assert.equal(effects.currentKey, keys[1]);

  // Wrap through the whole order back to the first layer.
  effects.update(SHUFFLE_SECONDS * (keys.length - 1));
  assert.equal(effects.currentKey, keys[0]);
});

test('shuffle skips disabled layers and moves on when the current one is disabled', () => {
  const effects = createEffectController();
  effects.setEnabled('network', false);
  effects.setShuffle(true);
  assert.equal(effects.currentKey, 'heatmap');
  effects.update(SHUFFLE_SECONDS);
  assert.equal(effects.currentKey, 'offside');

  effects.setEnabled('offside', false);
  assert.equal(effects.currentKey, 'analysis');
  assert.equal(effects.isVisible('offside'), false);
});

test('shuffle with no enabled layers shows nothing until one is enabled', () => {
  const effects = createEffectController();
  for (const key of keys) effects.setEnabled(key, false);
  effects.setShuffle(true);
  assert.equal(effects.currentKey, null);
  for (const key of keys) assert.equal(effects.isVisible(key), false);

  effects.setEnabled('visionCone', true);
  assert.equal(effects.currentKey, 'visionCone');
  assert.equal(effects.isVisible('visionCone'), true);
});

test('turning shuffle off restores the normal toggle visibility', () => {
  const effects = createEffectController();
  effects.setEnabled('heatmap', false);
  effects.setShuffle(true);
  assert.equal(effects.currentKey, 'network');
  assert.equal(effects.isVisible('heatmap'), false);

  effects.setShuffle(false);
  assert.equal(effects.shuffleActive, false);
  assert.equal(effects.currentKey, null);
  assert.equal(effects.isVisible('heatmap'), false);
  assert.equal(effects.isVisible('network'), true);
  assert.equal(effects.isVisible('visionCone'), true);
});
