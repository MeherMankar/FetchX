/**
 * Assembles the final extension package for the target browser.
 *
 * Usage:  node scripts/assemble.mjs chromium
 *         node scripts/assemble.mjs firefox
 *
 * Copies the built `dist/` files, icons, popup.html, and the
 * correct manifest.json into `dist-chromium/` or `dist-firefox/`.
 */

import { cpSync, mkdirSync, copyFileSync, rmSync, existsSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const __dir = dirname(fileURLToPath(import.meta.url));
const root  = join(__dir, "..");

const target = process.argv[2];
if (target !== "chromium" && target !== "firefox") {
  console.error("Usage: node assemble.mjs chromium|firefox");
  process.exit(1);
}

const outDir = join(root, `dist-${target}`);

// Clean
if (existsSync(outDir)) rmSync(outDir, { recursive: true });
mkdirSync(outDir, { recursive: true });

// dist/ JS + CSS
cpSync(join(root, "dist"), join(outDir, "dist"), { recursive: true });

// icons
cpSync(join(root, "icons"), join(outDir, "icons"), { recursive: true });

// popup.html
copyFileSync(join(root, "popup.html"), join(outDir, "popup.html"));

// manifest
const manifestSrc = target === "firefox"
  ? join(root, "manifest-firefox", "manifest.json")
  : join(root, "manifest.json");
copyFileSync(manifestSrc, join(outDir, "manifest.json"));

console.log(`\n✓ ${target} extension assembled → dist-${target}/`);
