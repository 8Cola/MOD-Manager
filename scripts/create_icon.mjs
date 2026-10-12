import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { Gamepad2 } from 'lucide-react';
import { Resvg } from '@resvg/resvg-js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
// Match the existing sidebar exactly: 45px blue square, 13px corners, 25px Lucide Gamepad2.
const controller = renderToStaticMarkup(React.createElement(Gamepad2, { size: 25, color: '#fff', strokeWidth: 2 }))
  .replace('<svg ', '<svg x="10" y="10" ');
const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="45" height="45" viewBox="0 0 45 45"><rect width="45" height="45" rx="13" fill="#3979f8"/>${controller}</svg>\n`;
const assets = path.join(root, 'src/assets');
const icons = path.join(root, 'src-tauri/icons');
fs.mkdirSync(assets, { recursive: true });
fs.mkdirSync(icons, { recursive: true });
fs.writeFileSync(path.join(assets, 'app-icon.svg'), svg);
const sizes = [16, 24, 32, 48, 64, 128, 256];
const frames = sizes.map(size => new Resvg(svg, { fitTo: { mode: 'width', value: size } }).render().asPng());
fs.writeFileSync(path.join(icons, 'icon.png'), frames.at(-1));
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
console.log('UI SVG and Windows icon generated from the exact same Lucide artwork.');
