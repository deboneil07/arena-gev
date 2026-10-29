// Layer presets. The preset is stored separately from the variant choices so
// a reload restores both the analyst's chosen view and the chosen skins.
import { EFFECTS } from './effects.js';

const STORAGE_KEY = 'gev.preset';
const ALL = Object.freeze(EFFECTS.map((effect) => effect.key));

export const PRESETS = Object.freeze({
  broadcast: Object.freeze(['heatmap', 'offside', 'analysis']),
  tactical: Object.freeze(['heatmap', 'network', 'offside', 'analysis', 'passingLanes', 'attackChannels', 'visionCone']),
  clean: Object.freeze([]),
  all: ALL,
});

export function isPreset(name) {
  return Object.prototype.hasOwnProperty.call(PRESETS, name);
}

export function presetKeys() {
  return Object.keys(PRESETS);
}

export function readStoredPreset() {
  try {
    const value = typeof localStorage !== 'undefined' ? localStorage.getItem(STORAGE_KEY) : null;
    return isPreset(value) ? value : null;
  } catch {
    return null;
  }
}

export function storePreset(name) {
  if (!isPreset(name)) return false;
  try {
    if (typeof localStorage !== 'undefined') localStorage.setItem(STORAGE_KEY, name);
  } catch { /* ignore */ }
  return true;
}

export function clearStoredPreset() {
  try {
    if (typeof localStorage !== 'undefined') localStorage.removeItem(STORAGE_KEY);
  } catch { /* ignore */ }
}
