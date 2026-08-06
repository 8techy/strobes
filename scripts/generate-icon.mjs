/**
 * Syncs the brand app icon into src-tauri/icons/source.png, then run:
 *
 *   node scripts/generate-icon.mjs
 *   npx tauri icon src-tauri/icons/source.png
 *
 * Source artwork lives in public/icon-dark.png (white mark on black).
 */

import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";

const SOURCE = resolve("public/icon-dark.png");
const OUTPUT = resolve("src-tauri/icons/source.png");

if (!existsSync(SOURCE)) {
  console.error(`missing brand icon: ${SOURCE}`);
  process.exit(1);
}

mkdirSync(dirname(OUTPUT), { recursive: true });
copyFileSync(SOURCE, OUTPUT);
console.log(`wrote ${OUTPUT}`);
console.log("next: npx tauri icon src-tauri/icons/source.png");
