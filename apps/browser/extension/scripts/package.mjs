/**
 * Packages a dist-chromium/ or dist-firefox/ folder into a ZIP file
 * with POSIX (forward-slash) paths — required by AMO and CWS validators.
 *
 * Usage:  node scripts/package.mjs chromium
 *         node scripts/package.mjs firefox
 *
 * Output: fetchx-<target>-<version>.zip in the extension root.
 *
 * Uses only Node built-ins + the JSZip-free approach via the
 * Compression Streams API (Node 18+). Falls back to a manual ZIP
 * writer using the ZIP local file header format with DEFLATE via zlib.
 */

import { createWriteStream, readFileSync, readdirSync, statSync } from "fs";
import { join, relative, dirname, posix } from "path";
import { fileURLToPath } from "url";
import { deflateRawSync } from "zlib";
import { createReadStream } from "fs";

const __dir = dirname(fileURLToPath(import.meta.url));
const root  = join(__dir, "..");

const target = process.argv[2];
if (target !== "chromium" && target !== "firefox") {
  console.error("Usage: node package.mjs chromium|firefox");
  process.exit(1);
}

// Read version from manifest
const manifestPath = join(root, `dist-${target}`, "manifest.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const version = manifest.version ?? "0.0.0";

const srcDir  = join(root, `dist-${target}`);
const outFile = join(root, `fetchx-${target}-${version}.zip`);

// Collect all files recursively
function collectFiles(dir) {
  const files = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      files.push(...collectFiles(full));
    } else {
      files.push(full);
    }
  }
  return files;
}

const allFiles = collectFiles(srcDir);

// Build ZIP manually — ZIP local file header + data descriptor + central directory
// This guarantees POSIX paths (forward slashes) regardless of OS.

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let k = 0; k < 8; k++) c = (c & 1) ? (0xEDB88320 ^ (c >>> 1)) : (c >>> 1);
    t[i] = c;
  }
  return t;
})();

function crc32(buf) {
  let crc = 0xFFFFFFFF;
  for (const b of buf) crc = CRC_TABLE[(crc ^ b) & 0xFF] ^ (crc >>> 8);
  return (crc ^ 0xFFFFFFFF) >>> 0;
}

function uint16LE(n) { const b = Buffer.alloc(2); b.writeUInt16LE(n, 0); return b; }
function uint32LE(n) { const b = Buffer.alloc(4); b.writeUInt32LE(n >>> 0, 0); return b; }

const entries = [];
let offset = 0;
const chunks = [];

for (const filePath of allFiles) {
  const name     = posix.normalize(relative(srcDir, filePath).replace(/\\/g, "/"));
  let data       = readFileSync(filePath);
  if (target === "chromium" && name === "manifest.json") {
    const packagedManifest = JSON.parse(data.toString("utf8"));
    delete packagedManifest.key;
    data = Buffer.from(`${JSON.stringify(packagedManifest, null, 2)}\n`);
  }
  const nameBytes = Buffer.from(name, "utf8");
  const compressed = deflateRawSync(data, { level: 9 });
  const crc      = crc32(data);

  // Local file header
  const localHeader = Buffer.concat([
    Buffer.from([0x50, 0x4B, 0x03, 0x04]), // signature
    uint16LE(20),           // version needed
    uint16LE(0),            // flags
    uint16LE(8),            // compression: DEFLATE
    uint16LE(0),            // mod time
    uint16LE(0),            // mod date
    uint32LE(crc),
    uint32LE(compressed.length),
    uint32LE(data.length),
    uint16LE(nameBytes.length),
    uint16LE(0),            // extra length
    nameBytes,
  ]);

  entries.push({
    name: nameBytes,
    crc,
    compressedSize: compressed.length,
    uncompressedSize: data.length,
    offset,
  });

  offset += localHeader.length + compressed.length;
  chunks.push(localHeader, compressed);
}

// Central directory
const cdChunks = [];
for (const e of entries) {
  const cd = Buffer.concat([
    Buffer.from([0x50, 0x4B, 0x01, 0x02]), // signature
    uint16LE(20),  // version made by
    uint16LE(20),  // version needed
    uint16LE(0),   // flags
    uint16LE(8),   // compression
    uint16LE(0),   // mod time
    uint16LE(0),   // mod date
    uint32LE(e.crc),
    uint32LE(e.compressedSize),
    uint32LE(e.uncompressedSize),
    uint16LE(e.name.length),
    uint16LE(0),   // extra
    uint16LE(0),   // comment
    uint16LE(0),   // disk start
    uint16LE(0),   // internal attr
    uint32LE(0),   // external attr
    uint32LE(e.offset),
    e.name,
  ]);
  cdChunks.push(cd);
}

const cdData   = Buffer.concat(cdChunks);
const cdOffset = offset;
const cdSize   = cdData.length;

// End of central directory record
const eocd = Buffer.concat([
  Buffer.from([0x50, 0x4B, 0x05, 0x06]),
  uint16LE(0), uint16LE(0),
  uint16LE(entries.length),
  uint16LE(entries.length),
  uint32LE(cdSize),
  uint32LE(cdOffset),
  uint16LE(0),
]);

const ws = createWriteStream(outFile);
for (const chunk of [...chunks, cdData, eocd]) ws.write(chunk);
ws.end(() => {
  const kb = Math.round(statSync(outFile).size / 1024);
  console.log(`\n✓ ${outFile} (${kb} KB)`);
});
