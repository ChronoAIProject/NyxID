import type { Plugin } from "vite"
import { existsSync, readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import postcss from "postcss"

const here = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(here, "..", "..")

/** CSS imports are inlined before Rollup sees their individual module IDs. */
export function wizardSourceFiles(moduleIds: Iterable<string>, root = repoRoot): string[] {
  const files = new Set<string>()
  function add(id: string) {
    if (id.includes("\0") || id.includes("node_modules") || !id.startsWith(root + path.sep) || files.has(id)) return
    files.add(id)
    if (path.extname(id) !== ".css") return
    postcss.parse(readFileSync(id, "utf8")).walkAtRules("import", rule => {
      const match = rule.params.match(/^(?:url\(\s*)?["']([^"']+)["']/)
      if (!match) return
      const imported = path.resolve(path.dirname(id), match[1].split(/[?#]/)[0])
      if (path.extname(imported) === ".css" && existsSync(imported)) add(imported)
    })
  }
  for (const id of moduleIds) add(id)
  return [...files].map(id => path.relative(root, id).split(path.sep).join("/")).sort()
}

/**
 * Emits `wizard.manifest`, a sorted newline-separated list of every
 * repo-relative source file Rollup or a CSS import traversed for the wizard.
 * Node-module dependencies are excluded (their bytes are already pinned
 * transitively by `frontend/package-lock.json`, which the CI freshness
 * check hashes separately).
 *
 * This manifest is the authoritative dep list for
 * `cli/tests/wizard_bundle_freshness.rs` — edits to any listed file are
 * caught because the freshness hash includes the file's contents. Files
 * outside the manifest + the extras list don't affect the bundle, so
 * editing them doesn't trip the check.
 */
export function wizardManifest(): Plugin {
  return {
    name: "wizard-manifest",
    apply: "build",
    generateBundle() {
      const files = wizardSourceFiles(this.getModuleIds())
      this.emitFile({
        type: "asset",
        fileName: "wizard.manifest",
        source: files.join("\n") + "\n",
      })
    },
  }
}
