import fs from "node:fs"
import path from "node:path"
import { createHash } from "node:crypto"
import type { Plugin } from "vite"
import { contentHash, htmlContents, singlefileContent } from "../scripts/html-composition.mjs"

const sha256 = (bytes: string | Buffer) => createHash("sha256").update(bytes).digest("hex")
const posix = (value: string) => value.split(path.sep).join("/")

function styleImports(file: string, root: string): string[] {
  const text = fs.readFileSync(file, "utf8").replace(/\/\*[\s\S]*?\*\//g, "")
  const imports = [...text.matchAll(/@import\s+(?:url\()?['"]([^'"]+)['"]\)?[^;]*;/g)]
  if (text.replace(/@import\s+(?:url\()?['"]([^'"]+)['"]\)?[^;]*;/g, "").includes("@import")) {
    throw new Error("Unmapped CSS import syntax")
  }
  return imports.map(([, name]) => {
    if (!name || /^(?:https?:|data:|\/)/.test(name)) throw new Error("Unmapped external CSS import")
    if (name.startsWith(".")) return path.resolve(path.dirname(file), name)
    const parts = name.split("/")
    const packageName = parts.splice(0, name.startsWith("@") ? 2 : 1).join("/")
    let parent = path.dirname(file)
    for (;;) {
      const directory = path.join(parent, "node_modules", packageName)
      const manifest = path.join(directory, "package.json")
      if (fs.existsSync(manifest)) {
        const pkg = JSON.parse(fs.readFileSync(manifest, "utf8"))
        const key = parts.length ? "./" + parts.join("/") : "."
        const exported = pkg.exports?.[key]
        const entry = typeof exported === "string" ? exported : exported?.style ?? (parts.length ? parts.join("/") : pkg.style)
        if (!entry || !entry.endsWith(".css")) throw new Error("Unmapped package CSS entry")
        const resolved = path.resolve(directory, entry)
        if (!resolved.startsWith(directory + path.sep)) throw new Error("Invalid package CSS entry")
        return resolved
      }
      if (parent === root || parent === path.dirname(parent)) break
      parent = path.dirname(parent)
    }
    throw new Error("CSS dependency is not installed")
  })
}

interface PackageIdentity {
  name: string
  version: string
  purl: string
  integrity: string
  lock_path: string
}
interface ModuleIdentity {
  id: string
  source_sha256?: string
  package?: PackageIdentity
  generated_by?: PackageIdentity
  rendered_length: number
}

/** Resolve an installed file to its exact lock entry, never the entire lock graph. */
export function moduleIdentity(raw: string, root: string, lock: {
  packages: Record<string, { name?: string; version?: string; integrity?: string }>
}): Omit<ModuleIdentity, "rendered_length"> {
  const id = raw.replace(/^\0/, "").split("?")[0]!
  const repo = path.dirname(root)
  if (id.includes(`${path.sep}node_modules${path.sep}`)) {
    let directory = path.dirname(id)
    while (directory !== path.dirname(directory)) {
      const manifest = path.join(directory, "package.json")
      const lockPath = posix(path.relative(root, directory))
      const locked = lock.packages[lockPath]
      if (locked) {
        const installed = JSON.parse(fs.readFileSync(manifest, "utf8"))
        const expectedName = locked.name ?? lockPath.slice(lockPath.lastIndexOf("node_modules/") + "node_modules/".length)
        if (!locked.integrity || !locked.version || installed.version !== locked.version ||
          installed.name !== expectedName) {
          throw new Error(`Unmapped or mismatched installed package: ${lockPath}`)
        }
        const identity: PackageIdentity = {
          name: installed.name, version: locked.version,
          purl: `pkg:npm/${installed.name.replace(/^@/, "%40")}@${locked.version}`,
          integrity: locked.integrity, lock_path: lockPath,
        }
        return {
          id: `${identity.purl}/${posix(path.relative(directory, id))}`,
          source_sha256: sha256(fs.readFileSync(id)), package: identity,
        }
      }
      directory = path.dirname(directory)
    }
    throw new Error("Included third-party module has no installed lock mapping")
  }
  if (path.isAbsolute(id) && id.startsWith(repo + path.sep)) {
    return { id: `repo:${posix(path.relative(repo, id))}`, source_sha256: sha256(fs.readFileSync(id)) }
  }
  // These are bundler-generated helpers, not files or arbitrary external code.
  // Unknown virtual modules require an explicit reviewed mapping.
  if (/^(?:vite\/(?:modulepreload-polyfill|preload-helper)(?:\.js)?|commonjsHelpers\.js|rolldown\/runtime\.js)$/.test(id)) {
    const name = id.startsWith("rolldown/") ? "rolldown" : "vite"
    const tool = lock.packages[`node_modules/${name}`]
    if (!tool?.version || !tool.integrity) throw new Error("Missing bundler tool identity")
    return { id: `generated:${id}`, generated_by: {
      name, version: tool.version, purl: `pkg:npm/${name}@${tool.version}`,
      integrity: tool.integrity, lock_path: `node_modules/${name}`,
    } }
  }
  throw new Error(`Included module has unknown origin; composition cannot be certified: ${JSON.stringify(raw)}`)
}

/** Capture rendered membership before singlefile removes its intermediate chunks. */
export function shippedComposition(component: "spa" | "credential-accept" | "wizard"): Plugin[] {
  let root = ""
  let lock: Parameters<typeof moduleIdentity>[2]
  const chunks = new Map<string, ModuleIdentity[]>()
  const rendered = new Set<string>()
  const inline: { kind: string; sha256: string; size: number; transform: string; path: string; source_block_sha256?: string; source?: string }[] = []
  const capture: Plugin = {
    name: "nyxid-singlefile-input-composition", apply: "build", enforce: "post",
    // Vite emits aggregate CSS in its normal generateBundle hook. Capture
    // after that hook and before singlefile's post hook deletes the assets.
    generateBundle: { handler(_options, bundle) {
      if (component !== "wizard") return
      for (const entry of Object.values(bundle)) {
        const kind = entry.type === "chunk" ? "script" : entry.fileName.endsWith(".css") ? "style" : null
        if (!kind) continue
        const code = entry.type === "chunk" ? entry.code : String(entry.source)
        const content = singlefileContent(kind, code)
        const tool = lock.packages["node_modules/vite-plugin-singlefile"]
        if (!tool?.version || !tool.integrity) throw new Error("Missing singlefile transform identity")
        inline.push({ kind, sha256: contentHash(content), size: Buffer.byteLength(content), path: "wizard.html", transform: `pkg:npm/vite-plugin-singlefile@${tool.version}` })
      }
    } },
  }
  const record: Plugin = {
    name: "nyxid-shipped-composition", apply: "build", enforce: "post",
    configResolved(config) {
      root = config.root
      lock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"))
    },
    // Vite minifies template <style> blocks through its HTML CSS proxy.
    // Record that build transformation separately from generated singlefile
    // code, with the original block identity and exact template source.
    transformIndexHtml: { order: "post", handler(html, context) {
      const source = fs.readFileSync(context.filename, "utf8")
      const original = htmlContents(source).inline.filter(block => block.kind === "style" || block.kind === "style-attribute")
      const transformed = htmlContents(html).inline.filter(block => block.kind === "style" || block.kind === "style-attribute")
      if (original.length !== transformed.length) throw new Error("Unmapped transformed HTML style")
      const tool = lock.packages["node_modules/vite"]
      if (!tool?.version || !tool.integrity) throw new Error("Missing HTML transform identity")
      for (const [index, block] of transformed.entries()) {
        if (block.kind !== original[index]!.kind) throw new Error("Unmapped transformed HTML style order")
        inline.push({ ...block, path: posix(path.relative(root, context.filename)),
          source: "repo:" + posix(path.relative(path.dirname(root), context.filename)),
          source_block_sha256: original[index]!.sha256, transform: `pkg:npm/vite@${tool.version}` })
      }
    } },
    renderChunk: {
      order: "post",
      handler(_code, chunk) {
        const modules = Object.entries(chunk.modules)
          .filter(([, info]) => info.renderedLength > 0)
          .map(([id, info]) => {
            rendered.add(id)
            return { ...moduleIdentity(id, root, lock), rendered_length: info.renderedLength }
          })
          .sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0)
        if (!modules.length) this.error("Executable chunk has no rendered module composition")
        chunks.set(chunk.name, modules)
        return null
      },
    },
    generateBundle: {
      order: "post",
      handler(_options, bundle) {
        if (!chunks.size) this.error("No rendered chunk composition was captured")
        const finalChunks = Object.values(bundle).filter((entry) => entry.type === "chunk")
        const styles = new Set<string>()
        const visitStyle = (file: string) => {
          if (styles.has(file)) return
          styles.add(file)
          for (const imported of styleImports(file, root)) visitStyle(imported)
        }
        for (const id of this.getModuleIds()) {
          if (id.endsWith(".css") && this.getModuleInfo(id)?.importers.some(importer => rendered.has(importer))) visitStyle(id)
        }
        const styleOutputs = component === "wizard" && styles.size ? ["wizard.html"] : Object.values(bundle)
          .filter(entry => entry.type === "asset" && entry.fileName.endsWith(".css")).map(entry => entry.fileName).sort()
        if (styleOutputs.length && !styles.size) this.error("Stylesheet output has no retained source attribution")
        if (component === "wizard" && finalChunks.length) this.error("Wizard composition requires final singlefile output")
        const outputs = component === "wizard"
          ? [...chunks].map(([name, modules]) => ({ path: "wizard.html", inlined_from: name, modules }))
          : finalChunks.map((chunk) => ({ path: chunk.fileName, modules:
            Object.entries(chunk.modules).filter(([, info]) => info.renderedLength > 0)
              .map(([id, info]) => ({ ...moduleIdentity(id, root, lock), rendered_length: info.renderedLength }))
              .sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0),
          }))
        this.emitFile({ type: "asset", fileName: "composition-graph.json", source: JSON.stringify({
          schema: "nyxid.rendered-composition.v1", component,
          lock_sha256: sha256(fs.readFileSync(path.join(root, "package-lock.json"))),
          chunks: outputs.sort((a, b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0),
          inline_candidates: inline,
          styles: { outputs: styleOutputs, sources: [...styles].sort().map(file => moduleIdentity(file, root, lock)),
            // Tailwind compiles app.css into generated utilities. These are
            // build provenance identities, separate from retained CSS sources.
            generators: styles.has(path.join(root, "src/app.css")) ? ["@tailwindcss/vite", "tailwindcss"].map(name => {
              const installed = lock.packages[`node_modules/${name}`]
              if (!installed?.version || !installed.integrity) throw new Error("Missing style generator identity")
              return { name, version: installed.version, purl: `pkg:npm/${name.replace(/^@/, "%40")}@${installed.version}`, integrity: installed.integrity }
            }) : [],
          },
        }, null, 2) + "\n" })
      },
    },
  }
  return [capture, record]
}
