// Generates the application icons: a blue "B" on the panel-blue background the interface uses for
// its side panels.
//
// A tiny PNG encoder and ICO packer are used rather than an image library. The artwork is flat
// geometry — a background, a stem, three bars — so nothing a full encoder offers would help, and
// this keeps the build free of extra tooling.
//
// The "B" is drawn as rectangles with rounded right-hand corners rather than from a font, so the
// result is identical at every size and does not depend on which fonts are installed. Windows wants
// an ICO holding several sizes, because the shell picks a different one for the taskbar, the file
// list and the large-icon view; scaling a single bitmap down to 16px would look muddy.
//
// Usage: node scripts/make-icons.mjs

import { deflateSync } from 'node:zlib';
import { mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const OUT = resolve(here, '..', 'crates', 'app', 'icons');
mkdirSync(OUT, { recursive: true });

/** The side-panel background from the light theme, so the icon matches the application. */
const BG = [0xf4, 0xf6, 0xfb, 0xff];
/** The light theme's accent, used literally: on this pale surface it reads as a vivid blue. */
const FG = [0x00, 0x00, 0xff, 0xff];

/** CRC32, required by the PNG container. */
const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  return table;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (const b of buf) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

/** One horizontal bar of the "B", with its right-hand corners rounded. */
function inBar(u, v, box, radius) {
  const { x0, x1, y0, y1 } = box;
  if (u < x0 || u > x1 || v < y0 || v > y1) return false;
  const dx = x1 - u;
  const dy = Math.min(v - y0, y1 - v);
  if (dx >= radius) return true;
  const cx = radius - dx;
  const cy = radius - dy;
  if (cx <= 0 || cy <= 0) return true;
  return cx * cx + cy * cy <= radius * radius;
}

/** The "B" as a stem plus three bars. */
function isLetter(u, v) {
  const stem = { x0: 0.3, x1: 0.42, y0: 0.24, y1: 0.76 };
  if (u >= stem.x0 && u <= stem.x1 && v >= stem.y0 && v <= stem.y1) return true;

  const top = { x0: 0.3, x1: 0.68, y0: 0.24, y1: 0.44 };
  const middle = { x0: 0.3, x1: 0.64, y0: 0.44, y1: 0.56 };
  const bottom = { x0: 0.3, x1: 0.68, y0: 0.56, y1: 0.76 };
  const radius = 0.09;

  return inBar(u, v, top, radius) || inBar(u, v, middle, radius) || inBar(u, v, bottom, radius);
}

/** Colours one pixel, supersampling so the curves are not jagged. */
function pixel(size, x, y) {
  const samples = 4;
  let hits = 0;
  for (let sy = 0; sy < samples; sy += 1) {
    for (let sx = 0; sx < samples; sx += 1) {
      const u = (x + (sx + 0.5) / samples) / size;
      const v = (y + (sy + 0.5) / samples) / size;
      if (isLetter(u, v)) hits += 1;
    }
  }
  const alpha = hits / (samples * samples);
  if (alpha === 0) return BG;
  const mix = (a, b) => Math.round(a * alpha + b * (1 - alpha));
  return [mix(FG[0], BG[0]), mix(FG[1], BG[1]), mix(FG[2], BG[2]), 255];
}

function encodePng(size) {
  const raw = Buffer.alloc((size * 4 + 1) * size);
  let p = 0;
  for (let y = 0; y < size; y += 1) {
    raw[p] = 0; // filter: none
    p += 1;
    for (let x = 0; x < size; x += 1) {
      const [r, g, b] = pixel(size, x, y);
      raw[p] = r;
      raw[p + 1] = g;
      raw[p + 2] = b;
      raw[p + 3] = 255;
      p += 4;
    }
  }

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // colour type: RGBA
  ihdr[10] = 0;
  ihdr[11] = 0;
  ihdr[12] = 0;

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** Packs PNG images into an ICO container (PNG-in-ICO, supported since Windows Vista). */
function encodeIco(images) {
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0);
  header.writeUInt16LE(1, 2); // type: icon
  header.writeUInt16LE(images.length, 4);

  const entries = [];
  let offset = 6 + images.length * 16;
  for (const { size, data } of images) {
    const entry = Buffer.alloc(16);
    entry[0] = size >= 256 ? 0 : size; // 0 means 256
    entry[1] = size >= 256 ? 0 : size;
    entry.writeUInt16LE(1, 4); // colour planes
    entry.writeUInt16LE(32, 6); // bits per pixel
    entry.writeUInt32LE(data.length, 8);
    entry.writeUInt32LE(offset, 12);
    entries.push(entry);
    offset += data.length;
  }

  return Buffer.concat([header, ...entries, ...images.map((i) => i.data)]);
}

// Sizes the shell actually asks for. 16px is not optional: it is what the file list and the
// taskbar's small mode use, and it is the size most likely to look wrong.
const SIZES = [16, 24, 32, 48, 64, 128, 256];
const pngs = SIZES.map((size) => ({ size, data: encodePng(size) }));

for (const { size, data } of pngs) {
  writeFileSync(resolve(OUT, `${size}x${size}.png`), data);
}
console.log(`wrote ${SIZES.length} PNG sizes: ${SIZES.join(', ')}`);

// Names tauri.conf.json and the bundler expect.
const bySize = new Map(pngs.map((p) => [p.size, p.data]));
writeFileSync(resolve(OUT, '32x32.png'), bySize.get(32));
writeFileSync(resolve(OUT, '128x128.png'), bySize.get(128));
writeFileSync(resolve(OUT, '128x128@2x.png'), bySize.get(256));
writeFileSync(resolve(OUT, 'icon.png'), bySize.get(256));

const ico = encodeIco(pngs);
writeFileSync(resolve(OUT, 'icon.ico'), ico);
console.log(`wrote icon.ico (${ico.length} bytes, ${pngs.length} sizes embedded)`);
