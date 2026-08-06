/**
 * Generates the source app icon as a PNG.
 *
 * Kept as a script rather than a committed binary so the icon is reproducible and
 * reviewable. Run it, then hand the output to `tauri icon` to produce the
 * per-platform sizes and the Windows .ico that tauri-build requires.
 *
 *   node scripts/generate-icon.mjs
 *   npx tauri icon src-tauri/icons/source.png
 *
 * The artwork is two glowing rings on a dark rounded square, echoing the
 * headlight rings that the app's default effects drive.
 */

import { deflateSync } from "node:zlib";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const SIZE = 1024;
const OUTPUT = resolve("src-tauri/icons/source.png");

// Palette matches src/styles.css so the icon and the UI agree.
const BACKGROUND_TOP = [0x16, 0x1a, 0x24];
const BACKGROUND_BOTTOM = [0x08, 0x09, 0x0c];
const BEAM = [0x56, 0xb6, 0xff];

const CORNER_RADIUS = SIZE * 0.18;
const RING_RADIUS = SIZE * 0.185;
const RING_THICKNESS = SIZE * 0.045;
const RING_CENTERS = [
  [SIZE * 0.345, SIZE * 0.5],
  [SIZE * 0.655, SIZE * 0.5],
];

/** Smooth 0..1 ramp, used for cheap antialiasing. */
function smoothstep(edge0, edge1, x) {
  const t = Math.min(1, Math.max(0, (x - edge0) / (edge1 - edge0)));
  return t * t * (3 - 2 * t);
}

/** Coverage of a rounded square, antialiased at the boundary. */
function roundedSquareCoverage(x, y) {
  const half = SIZE / 2;
  // Distance from the centre, measured in the rounded-rect metric.
  const dx = Math.abs(x - half) - (half - CORNER_RADIUS);
  const dy = Math.abs(y - half) - (half - CORNER_RADIUS);
  const outside =
    Math.hypot(Math.max(dx, 0), Math.max(dy, 0)) -
    CORNER_RADIUS +
    Math.min(Math.max(dx, dy), 0);
  return 1 - smoothstep(-1, 1, outside);
}

/** Ring brightness at a point: a crisp band plus an outward glow. */
function ringIntensity(x, y) {
  let best = 0;
  for (const [cx, cy] of RING_CENTERS) {
    const distanceToEdge = Math.abs(Math.hypot(x - cx, y - cy) - RING_RADIUS);
    const band = 1 - smoothstep(RING_THICKNESS / 2 - 1.5, RING_THICKNESS / 2 + 1.5, distanceToEdge);
    const glow = 0.38 * Math.exp(-distanceToEdge / (SIZE * 0.05));
    best = Math.max(best, Math.min(1, band + glow));
  }
  return best;
}

function buildPixels() {
  const pixels = Buffer.alloc(SIZE * SIZE * 4);

  for (let y = 0; y < SIZE; y += 1) {
    const verticalMix = y / (SIZE - 1);
    for (let x = 0; x < SIZE; x += 1) {
      const coverage = roundedSquareCoverage(x + 0.5, y + 0.5);
      const glow = ringIntensity(x + 0.5, y + 0.5);

      let r = BACKGROUND_TOP[0] + (BACKGROUND_BOTTOM[0] - BACKGROUND_TOP[0]) * verticalMix;
      let g = BACKGROUND_TOP[1] + (BACKGROUND_BOTTOM[1] - BACKGROUND_TOP[1]) * verticalMix;
      let b = BACKGROUND_TOP[2] + (BACKGROUND_BOTTOM[2] - BACKGROUND_TOP[2]) * verticalMix;

      // Screen the beam colour over the background so the glow reads as light
      // rather than as paint.
      r += (BEAM[0] - r) * glow;
      g += (BEAM[1] - g) * glow;
      b += (BEAM[2] - b) * glow;

      const offset = (y * SIZE + x) * 4;
      pixels[offset] = Math.round(r);
      pixels[offset + 1] = Math.round(g);
      pixels[offset + 2] = Math.round(b);
      pixels[offset + 3] = Math.round(coverage * 255);
    }
  }
  return pixels;
}

// -- Minimal PNG encoder ---------------------------------------------------

const CRC_TABLE = (() => {
  const table = new Int32Array(256);
  for (let n = 0; n < 256; n += 1) {
    let c = n;
    for (let k = 0; k < 8; k += 1) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[n] = c;
  }
  return table;
})();

function crc32(buffer) {
  let crc = -1;
  for (const byte of buffer) {
    crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ -1) >>> 0;
}

function chunk(type, data) {
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const typeAndData = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(typeAndData));
  return Buffer.concat([length, typeAndData, crc]);
}

function encodePng(pixels) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(SIZE, 0);
  header.writeUInt32BE(SIZE, 4);
  header[8] = 8; // bit depth
  header[9] = 6; // colour type: RGBA
  header[10] = 0; // deflate
  header[11] = 0; // adaptive filtering
  header[12] = 0; // no interlace

  // Each scanline is prefixed with its filter type; 0 means "none".
  const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
  for (let y = 0; y < SIZE; y += 1) {
    const rowStart = y * (SIZE * 4 + 1);
    raw[rowStart] = 0;
    pixels.copy(raw, rowStart + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
  }

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw, { level: 9 })),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

mkdirSync(dirname(OUTPUT), { recursive: true });
writeFileSync(OUTPUT, encodePng(buildPixels()));
console.log(`wrote ${OUTPUT} (${SIZE}x${SIZE})`);
