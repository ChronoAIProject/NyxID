import type { Plugin } from "vite"
import path from "node:path"
import { fileURLToPath } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(here, "..", "..")

/**
 * Emits `wizard.manifest`, a sorted newline-separated list of every
 * repo-relative source file Rollup traversed while building the wizard.
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
      const files = [...this.getModuleIds()]
        // Drop Rollup virtual modules (ids containing NUL).
        .filter((id) => !id.includes("\0"))
        // Drop anything installed — captured via lockfile hash.
        .filter((id) => !id.includes("node_modules"))
        // Keep only files inside the repo. External absolute paths
        // (e.g. Vite's client runtime) are noise for freshness.
        .filter((id) => id.startsWith(repoRoot + path.sep))
        .map((id) =>
          path.relative(repoRoot, id).split(path.sep).join("/"),
        )
        .sort()
      this.emitFile({
        type: "asset",
        fileName: "wizard.manifest",
        source: files.join("\n") + "\n",
      })
    },
  }
}
