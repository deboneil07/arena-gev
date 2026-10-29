import test from 'node:test';
import assert from 'node:assert/strict';
import { FIELD_THEMES, fieldTheme } from './fieldThemes.js';

test('field themes expose classic, broadcast and night palettes', () => {
  assert.deepEqual(Object.keys(FIELD_THEMES).sort(), ['broadcast', 'classic', 'night']);
  for (const theme of Object.values(FIELD_THEMES)) {
    assert.match(theme.base, /^#/);
    assert.match(theme.pitch, /^#/);
    assert.match(theme.stripe, /^rgba/);
  }
});

test('unknown field ids fall back to classic', () => {
  assert.equal(fieldTheme('nope'), FIELD_THEMES.classic);
  assert.equal(fieldTheme('night'), FIELD_THEMES.night);
});
