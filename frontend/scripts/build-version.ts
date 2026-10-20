import type { Plugin } from "vite";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { parseBuildVersion } from "../src/lib/build-version";

interface ModuleDependencies {
  readonly importedIds: readonly string[];
  readonly dynamicallyImportedIds: readonly string[];
}

export function assistantFingerprint(
  root: string,
  getModuleInfo: (id: string) => ModuleDependencies | null,
  buildInputs = "",
) {
  const files = new Set<string>();
  const visited = new Set<string>();
  const router = path.join(root, "src/router.tsx");
  const pages = path.join(root, "src/pages/lazy.ts");
  const main = path.join(root, "src/main.tsx");
  const assistant = path.join(root, "src/pages/assistant.tsx");
  if (!getModuleInfo(assistant) || !getModuleInfo(main)) {
    throw new Error("Frontend module graph was unavailable for the assistant fingerprint");
  }
  const visit = (id: string, assistantScope: boolean) => {
    const key = `${assistantScope}:${id}`;
    if (visited.has(key)) return;
    visited.add(key);
    const file = id.split("?")[0]!;
    if (file.includes(`${path.sep}node_modules${path.sep}`)) return;
    if (assistantScope && (file === router || file === pages || file === main)) {
      throw new Error("Assistant imports the route entrypoints; its update scope must be reviewed");
    }
    if (file.startsWith(path.join(root, "src") + path.sep)) {
      if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
        throw new Error("A frontend source in the assistant dependency graph could not be read");
      }
      files.add(file);
    }
    // Traverse the root shell, but stop at the barrel that loads other routes.
    if (file === pages) return;
    const info = getModuleInfo(id);
    for (const imported of info?.importedIds ?? []) visit(imported, assistantScope);
    for (const imported of info?.dynamicallyImportedIds ?? []) visit(imported, assistantScope);
  };
  visit(assistant, true);
  visit(main, false);
  for (const file of [
    "src/pages/lazy.ts", "src/router.tsx", "src/app.css", "index.html",
    "vite.config.ts", "src/lib/grok-audio-worklet.js", "package-lock.json",
  ]) {
    const absolute = path.join(root, file);
    if (fs.existsSync(absolute)) files.add(absolute);
  }
  const addScripts = (directory: string) => {
    if (!fs.existsSync(directory)) return;
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const file = path.join(directory, entry.name);
      if (entry.isDirectory()) addScripts(file);
      else if (/\.(?:ts|mjs|js)$/.test(entry.name)) files.add(file);
    }
  };
  addScripts(path.join(root, "scripts"));
  if (!files.has(path.join(root, "src/pages/assistant.tsx"))) {
    throw new Error("Assistant source was absent from the frontend build");
  }
  const hash = createHash("sha256");
  hash.update(buildInputs);
  hash.update("\0");
  for (const file of [...files].sort((a, b) => path.relative(root, a).localeCompare(path.relative(root, b)))) {
    hash.update(path.relative(root, file).replaceAll(path.sep, "/"));
    hash.update("\0");
    if (file === path.join(root, "package-lock.json")) {
      const lock = JSON.parse(fs.readFileSync(file, "utf8"));
      delete lock.version;
      if (lock.packages?.[""]) delete lock.packages[""].version;
      hash.update(JSON.stringify(lock));
    } else {
      hash.update(fs.readFileSync(file));
    }
    hash.update("\0");
  }
  return hash.digest("hex");
}

/** Publish the identity and initial assets of the SPA served by this image. */
export function buildVersion(buildId: string, commit: string | null): Plugin {
  let root = "";
  let assistant: string | null = null;
  let buildInputs = "";
  return {
    name: "nyxid-build-version",
    apply: "build",
    configResolved(config) {
      root = config.root;
      const defines = { ...config.define };
      delete defines.__BUILD_ID__;
      buildInputs = JSON.stringify({ env: config.env, defines });
    },
    buildEnd(error) {
      if (!error) assistant = assistantFingerprint(root, (id) => this.getModuleInfo(id), buildInputs);
    },
    transformIndexHtml() {
      return [
        { tag: "meta", attrs: { name: "nyxid-build-id", content: buildId } },
        { tag: "meta", attrs: { name: "nyxid-assistant-build-id", content: assistant ?? "" } },
      ];
    },
    generateBundle(_options, bundle) {
      const assets = new Set<string>();
      const visit = (fileName: string) => {
        if (assets.has(fileName)) return;
        const chunk = bundle[fileName];
        if (!chunk || chunk.type !== "chunk") return;
        assets.add(fileName);
        const metadata = chunk as typeof chunk & {
          viteMetadata?: { importedCss: Set<string> };
        };
        for (const css of metadata.viteMetadata?.importedCss ?? []) assets.add(css);
        for (const imported of chunk.imports) visit(imported);
      };
      for (const chunk of Object.values(bundle)) {
        if (chunk.type === "chunk" && chunk.isEntry && !chunk.isDynamicEntry) visit(chunk.fileName);
      }
      const version = parseBuildVersion({ buildId, commit, assistant, assets: [...assets].sort() });
      if (!version) this.error("Frontend build metadata exceeds the browser update contract");
      this.emitFile({
        type: "asset",
        fileName: "build-version.json",
        source: JSON.stringify(version),
      });
    },
  };
}
