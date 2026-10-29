import test from 'node:test';
import assert from 'node:assert/strict';
import { VARIANTS, variantKeys, variantsOf, getVariant, setVariant, resetVariants } from './variants.js';

test('variant catalog covers every analytical layer', () => {
  for (const key of ['visionCone', 'passLanes', 'runChannels', 'network']) {
    assert.ok(VARIANTS[key]?.length >= 2, key);
    if (key !== 'heatmap' && key !== 'network') assert.equal(VARIANTS[key][0].id, 'classic');
  }
});

test('variantsOf returns the catalog or empty', () => {
  assert.equal(variantsOf('nope').length, 0);
  assert.ok(variantsOf('passLanes').some((v) => v.id === 'pulse-free'));
});

test('recommended defaults are the softer, labeled variants', () => {
  assert.equal(getVariant('visionCone'), 'soft-wedge');
  assert.equal(getVariant('passLanes'), 'corridor');
  assert.equal(getVariant('runChannels'), 'lanes');
  assert.equal(getVariant('network'), 'outlet');
  assert.equal(getVariant('heatmap'), 'raster');
  assert.equal(getVariant('analysis'), 'classic');
  assert.equal(getVariant('field'), 'classic');
});

test('setVariant validates and persists selection', () => {
  assert.equal(setVariant('passLanes', 'classic'), true);
  assert.equal(getVariant('passLanes'), 'classic');
  assert.equal(setVariant('passLanes', 'nope'), false);
  assert.equal(getVariant('passLanes'), 'classic');
  assert.equal(setVariant('passLanes', 'classic'), false);
  resetVariants();
  assert.equal(getVariant('passLanes'), 'corridor');
});
