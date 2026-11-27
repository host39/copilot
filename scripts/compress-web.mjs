#!/usr/bin/env node
// Writes a gzip (`.gz`) and a Brotli (`.br`) copy next to every compressible file of a web
// bundle, so the server sends them as they are to clients that accept them (`ServeDir`'s
// precompressed files) and needs no compressor at run time. The image build runs it on
// `apps/web/dist`; the native shells embed the bundle without these copies.
//
//   node scripts/compress-web.mjs apps/web/dist

import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { extname, join } from "node:path";
import { brotliCompressSync, constants, gzipSync } from "node:zlib";

const COMPRESSIBLE = new Set([".css", ".html", ".js", ".json", ".svg", ".txt", ".webmanifest"]);
// Smaller files gain too little to be worth a second request path.
const MIN_BYTES = 1024;

const root = process.argv[2];
if (!root) {
  console.error("scripts.compress-web.usage node scripts/compress-web.mjs <dir>");
  process.exit(2);
}

let written = 0;
let saved = 0;
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      walk(path);
      continue;
    }
    if (!COMPRESSIBLE.has(extname(name))) continue;
    const bytes = readFileSync(path);
    if (bytes.length < MIN_BYTES) continue;
    const gzip = gzipSync(bytes, { level: 9 });
    const brotli = brotliCompressSync(bytes, {
      params: {
        [constants.BROTLI_PARAM_QUALITY]: constants.BROTLI_MAX_QUALITY,
        [constants.BROTLI_PARAM_SIZE_HINT]: bytes.length,
      },
    });
    writeFileSync(`${path}.gz`, gzip);
    writeFileSync(`${path}.br`, brotli);
    written += 1;
    saved += bytes.length - brotli.length;
  }
};
walk(root);
console.log(`scripts.compress-web.done files=${written} brotli_saved_bytes=${saved}`);
