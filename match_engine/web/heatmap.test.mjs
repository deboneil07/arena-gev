import test from 'node:test';
import assert from 'node:assert/strict';

import { createHeatmap } from './heatmap.js';

test('sampling places mass at the player cell and spreads to neighbours', () => {
  const heat = createHeatmap({ cols: 21, rows: 14 });
  heat.sample([{ x: 0, y: 0 }]);
  const midCol = 10;
  const midRow = 7;
  const centre = heat.grid[midRow * heat.cols + midCol];
  assert.ok(centre > 0);
  assert.equal(centre, heat.max);
  assert.ok(heat.grid[midRow * heat.cols + midCol + 1] > 0);
  assert.ok(heat.grid[(midRow + 1) * heat.cols + midCol] > 0);
});

test('the kernel maps world y to screen-oriented rows', () => {
  const heat = createHeatmap({ cols: 21, rows: 14 });
  heat.sample([{ x: 0, y: 30 }]);
  let row = -1;
  for (let r = 0; r < heat.rows; r++) {
    for (let c = 0; c < heat.cols; c++) {
      if (heat.grid[r * heat.cols + c] > 0) row = row === -1 ? r : row;
    }
  }
  assert.ok(row >= 0 && row <= 2, `top-of-pitch sample landed in row ${row}`);
});

test('reset clears the grid and the maximum', () => {
  const heat = createHeatmap();
  heat.sample([{ x: 10, y: 10 }]);
  const v = heat.version;
  heat.reset();
  assert.equal(heat.max, 0);
  assert.ok(heat.grid.every((v) => v === 0));
  assert.ok(heat.version > v);
});

test('an empty field reports max 0 and ignores non-finite points', () => {
  const heat = createHeatmap();
  assert.equal(heat.max, 0);
  heat.sample([{ x: NaN, y: 0 }, { x: 0, y: Infinity }]);
  assert.equal(heat.max, 0);
});

test('separate instances do not share density', () => {
  const a = createHeatmap();
  const b = createHeatmap();
  a.sample([{ x: 0, y: 0 }], 2);
  assert.ok(a.max > 0);
  assert.equal(b.max, 0);
});
