/**
 * Mock scenario footprint gate.
 *
 * The assistant's scripted mock scenarios used to be kept out of production
 * builds entirely by an `import.meta.env.DEV` module boundary, and this script
 * asserted the symbols were absent from `dist/`. They now ship behind the
 * platform feature flag `experimental:assistant-mock-scenarios` (off by
 * default, admin-toggled at runtime), so "absent from dist" is no longer the
 * invariant — the code has to be there for an operator to switch on.
 *
 * What must still hold is that nobody pays for it. Every mock module has to
 * stay in an async chunk reachable **only** through a dynamic import, so a
 * session without the flag never fetches the interceptor, the scenario engine,
 * the scripted config, or the persisted store. Rollup folding any of them into
 * the assistant page chunk or a shared vendor chunk would silently break that
 * and is exactly what this gate catches.
 *
 * Checks:
 *   R1 the flag-gated modules are present as their own manifest entries (the
 *      gate is not passing vacuously against a build that dropped them);
 *   R2 no chunk outside the mock set statically imports a chunk inside it;
 *   R3 nothing `dist/index.html` references at boot is a mock chunk;
 *   R4 the separate credential-accept entry carries none of it.
 */
import { readFile, readdir, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const frontendRoot = fileURLToPath(new URL("../", import.meta.url));
const distRoot = path.join(frontendRoot, "dist");
const credentialAcceptRoot = path.join(distRoot, "credential-accept");
const manifestPath = path.join(distRoot, ".vite", "manifest.json");
const indexHtmlPath = path.join(distRoot, "index.html");

/**
 * Dynamic-import entry points of the mock layer. Vite always keys these by
 * source path in the manifest because `pages/assistant.tsx` and
 * `lib/assistant/transport.ts` reach them through `import()`.
 *
 * Renaming or removing one of these is a deliberate edit — update this list
 * and the reasoning above with it.
 */
const requiredLazyModules = [
  "src/lib/assistant/scenario-intercept-transport.ts",
  "src/stores/assistant-mock-scenarios-store.ts",
  "src/components/assistant/mock-scenarios-action.tsx",
];

/** The page that owns the flag gate; it must not carry the payload itself. */
const assistantPageModule = "src/pages/assistant.tsx";

/**
 * Symbols that only appear inside built mock scenario code and survive
 * minification: the mock id prefix carried by the engine and scripted config,
 * and the persisted store's localStorage key. The graph checks below prove the
 * lazy entry points are dynamic-only; these catch the other shape of the same
 * regression, where a mock module's *contents* get merged into an eager chunk.
 */
const forbiddenSymbols = ["mockchat-", "mockscenarios"];

async function filesBelow(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await filesBelow(entryPath)));
    } else if (entry.isFile()) {
      files.push(entryPath);
    }
  }
  return files;
}

let credentialAcceptStat;
try {
  credentialAcceptStat = await stat(credentialAcceptRoot);
} catch {
  throw new Error(
    "Production build is missing dist/credential-accept; the full build chain did not run.",
  );
}
if (!credentialAcceptStat.isDirectory()) {
  throw new Error("dist/credential-accept exists but is not a directory.");
}

let manifest;
try {
  manifest = JSON.parse(await readFile(manifestPath, "utf8"));
} catch {
  throw new Error(
    `Production build is missing ${path.relative(frontendRoot, manifestPath)}; ` +
      "the mock footprint gate needs vite's `build.manifest` to read the import graph.",
  );
}

// ── R1: the flag-gated modules shipped as their own manifest entries ──
// A module loses its manifest entry when it stops being a dynamic-import
// boundary — which is exactly what adding a static import to it does, so this
// is usually the first thing to fire on a regression.
const missing = requiredLazyModules.filter((key) => !manifest[key]);
if (missing.length > 0) {
  throw new Error(
    "Flag-gated mock scenario modules have no chunk of their own — they were " +
      "folded into an eager chunk (check for a new static import of them), or " +
      `this script's module list is stale:\n${missing.join("\n")}`,
  );
}

const violations = [];
const lazyModules = new Set(requiredLazyModules);

// ── R2: no chunk statically reaches a lazy entry point ──
// Walk `imports` (static only — `dynamicImports` is deliberately not followed)
// from every other manifest entry. Anything the walk lands on is downloaded
// whenever its importer is, so a mock entry point turning up here means it is
// no longer flag-gated in practice.
const staticallyReachable = new Set();
const pending = Object.keys(manifest).filter((key) => !lazyModules.has(key));
while (pending.length > 0) {
  const key = pending.pop();
  for (const imported of manifest[key]?.imports ?? []) {
    if (staticallyReachable.has(imported)) continue;
    staticallyReachable.add(imported);
    pending.push(imported);
  }
}
for (const key of requiredLazyModules) {
  if (staticallyReachable.has(key)) {
    violations.push(
      `${key} is statically imported — mock scenario code must be reachable only via dynamic import`,
    );
  }
}

// ── R3: no mock payload inside an eagerly loaded chunk ──
// index.html carries the entry plus a modulepreload for its whole static
// graph, so this covers everything the app fetches before any route renders.
// The assistant page chunk is checked too: opening /assistant without the flag
// must not download the engine either.
const indexHtml = await readFile(indexHtmlPath, "utf8");
const eagerFiles = new Set(
  [...indexHtml.matchAll(/assets\/[A-Za-z0-9._-]+\.js/g)].map(
    (match) => match[0],
  ),
);
const assistantChunk = manifest[assistantPageModule]?.file;
if (!assistantChunk) {
  throw new Error(
    `Production build has no manifest entry for ${assistantPageModule}; ` +
      "the mock footprint gate cannot check the assistant page chunk.",
  );
}
eagerFiles.add(assistantChunk);
for (const relative of eagerFiles) {
  const contents = await readFile(path.join(distRoot, relative));
  for (const symbol of forbiddenSymbols) {
    if (contents.includes(Buffer.from(symbol))) {
      violations.push(`${relative} is loaded eagerly and contains ${symbol}`);
    }
  }
}

// ── R4: the credential-accept entry shares none of it ──
for (const file of await filesBelow(credentialAcceptRoot)) {
  const contents = await readFile(file);
  for (const symbol of forbiddenSymbols) {
    if (contents.includes(Buffer.from(symbol))) {
      violations.push(`${path.relative(distRoot, file)} contains ${symbol}`);
    }
  }
}

if (violations.length > 0) {
  throw new Error(
    `Mock scenario code escaped its lazy boundary:\n${violations.join("\n")}`,
  );
}

process.stdout.write(
  `Mock scenario footprint assertion passed; ${requiredLazyModules.length} flag-gated ` +
    "module(s) are dynamic-import only and credential-accept output is present.\n",
);
