export const VARIANTS = Object.freeze({
  visionCone: Object.freeze([
    Object.freeze({ id: 'classic', label: 'Classic sector' }),
    Object.freeze({ id: 'soft-wedge', label: 'Soft wedge' }),
    Object.freeze({ id: 'spotlight', label: 'Spotlight' }),
    Object.freeze({ id: 'minimal', label: 'Minimal outline' }),
  ]),
  passLanes: Object.freeze([
    Object.freeze({ id: 'classic', label: 'Classic bands' }),
    Object.freeze({ id: 'pulse-free', label: 'No glowing dots' }),
    Object.freeze({ id: 'corridor', label: 'Corridor + target chips' }),
    Object.freeze({ id: 'ghost-trail', label: 'Ghost trail' }),
  ]),
  runChannels: Object.freeze([
    Object.freeze({ id: 'classic', label: 'Arrows (legacy)' }),
    Object.freeze({ id: 'lanes', label: 'Lanes, no arrows' }),
    Object.freeze({ id: 'heat-landing', label: 'Landing blobs' }),
    Object.freeze({ id: 'ghosts', label: 'Player trails' }),
    Object.freeze({ id: 'minimal', label: 'Thin trace' }),
  ]),
  network: Object.freeze([
    Object.freeze({ id: 'outlet', label: 'Outlet per player' }),
    Object.freeze({ id: 'mesh', label: 'Full mesh' }),
  ]),
  heatmap: Object.freeze([
    Object.freeze({ id: 'raster', label: 'Raster' }),
    Object.freeze({ id: 'hexbin', label: 'Hex bins' }),
  ]),
  analysis: Object.freeze([
    Object.freeze({ id: 'classic', label: 'Classic' }),
    Object.freeze({ id: 'minimal', label: 'Minimal' }),
  ]),
  field: Object.freeze([
    Object.freeze({ id: 'classic', label: 'Classic' }),
    Object.freeze({ id: 'broadcast', label: 'Broadcast' }),
    Object.freeze({ id: 'night', label: 'Night' }),
  ]),
});

const STORAGE_PREFIX = 'gev.variant.';
// Recommended defaults: the softened cone, the labeled pass corridor and the
// arrow-free run lane. Classic remains one click away in the lab.
const DEFAULTS = Object.freeze({
  visionCone: 'soft-wedge',
  network: 'outlet',
  heatmap: 'raster',
  passLanes: 'corridor',
  runChannels: 'lanes',
});
const current = new Map(Object.keys(VARIANTS).map((k) => [k, DEFAULTS[k] ?? 'classic']));

function readStored(key) {
  try {
    const v = (typeof localStorage !== 'undefined') ? localStorage.getItem(STORAGE_PREFIX + key) : null;
    if (v && VARIANTS[key]?.some((o) => o.id === v)) return v;
  } catch { /* ignore */ }
  return null;
}

for (const key of Object.keys(VARIANTS)) {
  const stored = readStored(key);
  if (stored) current.set(key, stored);
}

export function variantKeys() { return Object.keys(VARIANTS); }
export function variantsOf(layer) { return VARIANTS[layer] ?? []; }
export function getVariant(layer) { return current.get(layer) ?? 'classic'; }
export function setVariant(layer, id) {
  if (!VARIANTS[layer]) return false;
  if (!VARIANTS[layer].some((o) => o.id === id)) return false;
  if (current.get(layer) === id) return false;
  current.set(layer, id);
  try {
    if (typeof localStorage !== 'undefined') localStorage.setItem(STORAGE_PREFIX + layer, id);
  } catch { /* ignore */ }
  return true;
}
export function resetVariants() {
  for (const key of Object.keys(VARIANTS)) {
    current.set(key, DEFAULTS[key] ?? 'classic');
    try { if (typeof localStorage !== 'undefined') localStorage.removeItem(STORAGE_PREFIX + key); } catch { /* ignore */ }
  }
}
