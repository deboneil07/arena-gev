// Pitch palettes for the Variant lab field selector. The renderer reads these
// every frame, so switching a field theme never touches the analysis maths.
export const FIELD_THEMES = Object.freeze({
  classic: Object.freeze({
    base: '#14261a',
    pitch: '#1a3322',
    stripe: 'rgba(255,255,255,0.025)',
  }),
  broadcast: Object.freeze({
    base: '#17351f',
    pitch: '#2a5a2a',
    stripe: 'rgba(255,255,255,0.045)',
  }),
  night: Object.freeze({
    base: '#07110b',
    pitch: '#0a1a12',
    stripe: 'rgba(120,255,180,0.035)',
  }),
});

export function fieldTheme(id) {
  return FIELD_THEMES[id] ?? FIELD_THEMES.classic;
}
