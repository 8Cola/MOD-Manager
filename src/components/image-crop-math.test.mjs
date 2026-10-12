import { test } from 'node:test';
import assert from 'node:assert/strict';
import { boundPan, cropRectangle } from './image-crop-math.ts';

test('landscape image starts with a centered square and cannot expose blank edges', () => {
  assert.deepEqual(cropRectangle(1600, 900, 1, { x: 0, y: 0 }), { x: 350, y: 0, size: 900 });
  assert.deepEqual(boundPan(1600, 900, 1, { x: 9, y: -9 }), { x: 350 / 900, y: 0 });
});
test('portrait and zoomed crops remain inside the original image at every corner', () => {
  for (const [width, height] of [[800, 1600], [1600, 800], [800, 800]]) {
    for (const zoom of [1, 1.5, 4]) {
      for (const x of [-5, 5]) for (const y of [-5, 5]) {
        const crop = cropRectangle(width, height, zoom, { x, y });
        assert.ok(crop.x >= -1e-8 && crop.y >= -1e-8);
        assert.ok(crop.x + crop.size <= width + 1e-8);
        assert.ok(crop.y + crop.size <= height + 1e-8);
        assert.equal(crop.size, Math.min(width, height) / zoom);
      }
    }
  }
});
test('moving the displayed image right selects pixels further left', () => {
  assert.deepEqual(cropRectangle(1600, 900, 2, { x: 0.5, y: 0 }), { x: 350, y: 225, size: 450 });
});
