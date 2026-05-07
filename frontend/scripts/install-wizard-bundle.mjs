#!/usr/bin/env node
/**
 * Runs after `vite build --config vite.wizard.config.ts` as part of the
 * `build:wizard` npm script. Copies the built bundle into the CLI crate
 * and writes a freshness hash so `cli/tests/wizard_bundle_freshness.rs`
 * can verify that the committed bundle matches the current source
 * closure without having to rebuild in CI.
 *
 * Hash inputs (must stay in lockstep with the Rust verifier):
 *   1. Every file listed in `dist-wizard/wizard.manifest` (path + contents).
 *   2. A fixed list of extras that affect the bundle but aren't in
 *      Vite's module graph (lockfile, vite config, entry HTML, this
 *      plugin, Node version pin).
 *   3. The manifest file's own contents (catches reorder/add/delete).
 *
 * Each field is separated by a NUL byte so `{path=ab, contents=cd}`
 * can't collide with `{path=a, contents=bcd}`.
 */
import crypto from "node:crypto"
import fs from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const FRONTEND = path.resolve(here, "..")
const REPO = path.resolve(FRONTEND, "..")

const DIST_HTML = path.join(FRONTEND, "dist-wizard", "wizard.html")
const DIST_MANIFEST = path.join(FRONTEND, "dist-wizard", "wizard.manifest")
const OUT_ASSETS_DIR = path.join(REPO, "cli", "src", "wizard", "assets")
const OUT_BUNDLE = path.join(OUT_ASSETS_DIR, "index.html")
const OUT_META_DIR = path.join(REPO, "cli", "src", "wizard", "bundle-meta")
const OUT_MANIFEST = path.join(OUT_META_DIR, "index.manifest")
const OUT_HASH = path.join(OUT_META_DIR, "index.hash")

const STATIC_ASSETS = [
  ["frontend/public/nyxid-wordmark.svg", "nyxid-wordmark.svg"],
  ["frontend/public/favicon.ico", "favicon.ico"],
]

const EXTRAS = [
  "frontend/package-lock.json",
  "frontend/vite.wizard.config.ts",
  "frontend/wizard.html",
  "frontend/vite-plugins/wizard-manifest.ts",
  ".node-version",
]

function must(p) {
  if (!fs.existsSync(p)) {
    console.error(`install-wizard-bundle: missing ${p}`)
    process.exit(1)
  }
  return p
}

const manifestBytes = fs.readFileSync(must(DIST_MANIFEST))
const manifestText = manifestBytes.toString("utf8")
const files = manifestText.split("\n").filter((l) => l.length > 0)

const h = crypto.createHash("sha256")
const NUL = Buffer.from([0])

for (const file of files) {
  const abs = path.join(REPO, file)
  h.update(file, "utf8")
  h.update(NUL)
  h.update(fs.readFileSync(must(abs)))
  h.update(NUL)
}
for (const file of EXTRAS) {
  const abs = path.join(REPO, file)
  h.update(file, "utf8")
  h.update(NUL)
  h.update(fs.readFileSync(must(abs)))
  h.update(NUL)
}
h.update(manifestBytes)

const digest = h.digest("hex")

fs.mkdirSync(OUT_ASSETS_DIR, { recursive: true })
fs.mkdirSync(OUT_META_DIR, { recursive: true })
fs.copyFileSync(must(DIST_HTML), OUT_BUNDLE)
for (const [source, target] of STATIC_ASSETS) {
  fs.copyFileSync(must(path.join(REPO, source)), path.join(OUT_ASSETS_DIR, target))
}
fs.writeFileSync(OUT_MANIFEST, manifestText)
fs.writeFileSync(OUT_HASH, digest + "\n")

const relBundle = path.relative(REPO, OUT_BUNDLE)
const relManifest = path.relative(REPO, OUT_MANIFEST)
const relHash = path.relative(REPO, OUT_HASH)
console.log(`install-wizard-bundle: wrote ${relBundle}`)
for (const [, target] of STATIC_ASSETS) {
  console.log(
    `install-wizard-bundle: wrote ${path.relative(REPO, path.join(OUT_ASSETS_DIR, target))}`,
  )
}
console.log(`install-wizard-bundle: wrote ${relManifest} (${files.length} files)`)
console.log(`install-wizard-bundle: wrote ${relHash} (${digest.slice(0, 12)}…)`)
