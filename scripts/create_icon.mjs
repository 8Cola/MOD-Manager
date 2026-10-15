import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Resvg } from '@resvg/resvg-js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const assets = path.join(root, 'src/assets');
const icons = path.join(root, 'src-tauri/icons');
// Canonical hand-drawn vectors: UI and every Windows frame use one source.
const svg = fs.readFileSync(path.join(assets, 'app-icon.svg'), 'utf8');
if (/<image\b|data:image|https?:\/\/(?!www\.w3\.org)/i.test(svg)) throw new Error('Logo must be self-contained vector artwork');
fs.mkdirSync(icons, { recursive: true });
const sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
const frames = sizes.map(size => new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render().asPng());
fs.writeFileSync(path.join(icons, 'icon.png'), new Resvg(svg, { fitTo:{mode:'width',value:512} }).render().asPng());
const header = Buffer.alloc(6 + 16 * frames.length);
header.writeUInt16LE(1, 2);
header.writeUInt16LE(frames.length, 4);
let offset = header.length;
frames.forEach((frame, i) => {
  const entry = 6 + i * 16;
  header[entry] = header[entry + 1] = sizes[i] === 256 ? 0 : sizes[i];
  header.writeUInt16LE(1, entry + 4);
  header.writeUInt16LE(32, entry + 6);
  header.writeUInt32LE(frame.length, entry + 8);
  header.writeUInt32LE(offset, entry + 12);
  offset += frame.length;
});
fs.writeFileSync(path.join(icons, 'icon.ico'), Buffer.concat([header, ...frames]));
console.log('Glass cube logo: Windows ICO (10 sizes) and PNG generated from the canonical UI SVG.');
