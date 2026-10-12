export type CropPan = { x: number; y: number };
export function boundPan(width: number, height: number, zoom: number, pan: CropPan): CropPan {
  const shortest = Math.min(width, height);
  const limitX = (width * zoom - shortest) / (shortest * 2);
  const limitY = (height * zoom - shortest) / (shortest * 2);
  return { x: Math.max(-limitX, Math.min(limitX, pan.x)) || 0, y: Math.max(-limitY, Math.min(limitY, pan.y)) || 0 };
}
export function cropRectangle(width: number, height: number, zoom: number, pan: CropPan): { x: number; y: number; size: number } {
  const bounded = boundPan(width, height, zoom, pan);
  const size = Math.min(width, height) / zoom;
  return { x: Math.max(0, Math.min(width - size, (width - size) / 2 - bounded.x * size)), y: Math.max(0, Math.min(height - size, (height - size) / 2 - bounded.y * size)), size };
}
