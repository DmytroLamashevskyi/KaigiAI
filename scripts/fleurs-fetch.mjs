// Downloads a small benchmark sample of Google FLEURS (CC-BY-4.0) into
// bench/data/ — see docs/PROJECT.md §10.19. Not committed (bench/ is ignored).
//
//   node scripts/fleurs-fetch.mjs [--langs ru,en,ja,zh,ko,id,mn] [--limit 40]
//
// FLEURS sentences are parallel across languages (same FLoRes `id`), so for each
// chosen utterance the reference text in EVERY language is known — that gives
// both the STT reference (same language) and the translation references.
//
// The audio archives are hundreds of MB per language, so instead of downloading
// them the script streams `dev.tar.gz` and stops after `--limit` utterances
// whose sentence id exists in all requested languages (a few MB per language).

import { createWriteStream, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { Readable } from "node:stream";
import { fileURLToPath } from "node:url";
import { createGunzip } from "node:zlib";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const BASE = "https://huggingface.co/datasets/google/fleurs/resolve/main/data";

// App language code → FLEURS config.
const FLEURS = {
  en: "en_us",
  ru: "ru_ru",
  ja: "ja_jp",
  zh: "cmn_hans_cn",
  ko: "ko_kr",
  id: "id_id",
  mn: "mn_mn",
  es: "es_419",
  fr: "fr_fr",
  de: "de_de",
  it: "it_it",
  pt: "pt_br",
  uk: "uk_ua",
  pl: "pl_pl",
  tr: "tr_tr",
  ar: "ar_eg",
};

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
}

const langs = arg("langs", "ru,en,ja,zh,ko,id,mn").split(",").map((s) => s.trim());
const limit = Number(arg("limit", "40"));
for (const l of langs) {
  if (!FLEURS[l]) throw new Error(`unknown language "${l}" (known: ${Object.keys(FLEURS).join(", ")})`);
}

async function get(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}: ${url}`);
  return res;
}

/** id → raw transcription, from every split's TSV (columns: id, file, raw, …). */
async function sentences(config) {
  const byId = new Map();
  const files = new Map(); // wav file name → id, to match archive members
  for (const split of ["dev", "test", "train"]) {
    const text = await (await get(`${BASE}/${config}/${split}.tsv`)).text();
    for (const line of text.split("\n")) {
      const [id, file, raw] = line.split("\t");
      if (!id || !raw) continue;
      if (!byId.has(id)) byId.set(id, raw.trim());
      if (split === "dev") files.set(file, id);
    }
  }
  return { byId, files };
}

/** Stream a .tar.gz and call `onFile(name, bytes)` per regular file until it
 *  returns false. Minimal ustar reader: 512-byte header, size in octal at 124. */
async function streamTar(url, onFile) {
  const res = await get(url);
  const gunzip = Readable.fromWeb(res.body).pipe(createGunzip());
  let buf = Buffer.alloc(0);
  let want = null; // { name, size, padded }
  for await (const chunk of gunzip) {
    buf = Buffer.concat([buf, chunk]);
    for (;;) {
      if (!want) {
        if (buf.length < 512) break;
        const header = buf.subarray(0, 512);
        buf = buf.subarray(512);
        if (header.every((b) => b === 0)) continue; // end-of-archive padding
        const name = header.subarray(0, 100).toString("utf8").replace(/\0.*$/s, "");
        const size = parseInt(header.subarray(124, 136).toString("utf8").replace(/\0.*$/s, "").trim() || "0", 8);
        const type = String.fromCharCode(header[156] || 48);
        want = { name, size, padded: Math.ceil(size / 512) * 512, file: type === "0" };
      }
      if (buf.length < want.padded) break;
      const body = buf.subarray(0, want.size);
      buf = buf.subarray(want.padded);
      const w = want;
      want = null;
      if (w.file && !onFile(w.name.split("/").pop(), body)) {
        gunzip.destroy();
        return;
      }
    }
  }
}

console.log(`FLEURS sample: ${langs.join(", ")} × ${limit}`);
const meta = {};
for (const l of langs) {
  process.stdout.write(`  ${l}: transcripts… `);
  meta[l] = await sentences(FLEURS[l]);
  console.log(`${meta[l].byId.size} sentences`);
}
// Sentence ids present in every language → every pair has a reference.
const common = new Set([...meta[langs[0]].byId.keys()].filter((id) => langs.every((l) => meta[l].byId.has(id))));
console.log(`  sentences shared by all languages: ${common.size}`);

const manifest = { source: "google/fleurs (CC-BY-4.0)", langs, items: {}, refs: {} };
for (const l of langs) {
  const dir = join(root, "bench", "data", l);
  mkdirSync(dir, { recursive: true });
  const items = [];
  const seen = new Set();
  process.stdout.write(`  ${l}: audio… `);
  await streamTar(`${BASE}/${FLEURS[l]}/audio/dev.tar.gz`, (name, bytes) => {
    const id = meta[l].files.get(name);
    if (!id || !common.has(id) || seen.has(id)) return true; // one speaker per sentence
    seen.add(id);
    const file = `${l}/${id}.wav`;
    createWriteStream(join(root, "bench", "data", file)).end(bytes);
    items.push({ id, file });
    return items.length < limit;
  });
  console.log(`${items.length} utterances`);
  manifest.items[l] = items;
}
// References for every downloaded sentence in every language.
const ids = new Set(Object.values(manifest.items).flat().map((it) => it.id));
for (const id of ids) {
  manifest.refs[id] = Object.fromEntries(langs.map((l) => [l, meta[l].byId.get(id)]));
}
writeFileSync(join(root, "bench", "data", "manifest.json"), JSON.stringify(manifest, null, 2));
console.log(`bench/data/manifest.json: ${ids.size} sentences`);
