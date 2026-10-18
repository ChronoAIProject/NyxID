import { afterEach, describe, expect, it } from "vitest"
import { build } from "vite"
import tailwindcss from "@tailwindcss/vite"
import { viteSingleFile } from "vite-plugin-singlefile"
import fs from "node:fs"
import os from "node:os"
import path from "node:path"
import { createHash } from "node:crypto"
import { moduleIdentity, shippedComposition } from "../vite-plugins/shipped-composition"
import { htmlContents } from "../scripts/html-composition.mjs"
// @ts-expect-error Node build utility deliberately has no runtime TS dependency.
import { seal } from "../scripts/seal-shipped-composition.mjs"

const root = path.resolve(import.meta.dirname, "..")
const hash = (bytes: string | Buffer) => createHash("sha256").update(bytes).digest("hex")
const lock = JSON.parse(fs.readFileSync(path.join(root, "package-lock.json"), "utf8"))
const temporary: string[] = []
const temp = () => { const dir = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-composition-")); temporary.push(dir); return dir }
afterEach(() => temporary.splice(0).forEach(dir => fs.rmSync(dir, { recursive: true, force: true })))

describe("actual rendered composition", () => {
  it("records included package code, omits a tree-shaken dependency, and removes host paths", async () => {
    const output = temp()
    await build({ configFile: false, root, logLevel: "silent", plugins: [tailwindcss(), shippedComposition("spa")],
      build: { outDir: output, emptyOutDir: true, rollupOptions: { input: path.join(root, "test/fixtures/composition-entry.js") } },
    })
    const bytes = fs.readFileSync(path.join(output, "composition-graph.json"), "utf8")
    const graph = JSON.parse(bytes)
    const included = graph.chunks.flatMap((chunk: { modules: { package?: { name: string } }[] }) => chunk.modules)
    expect(included.some((module: { package?: { name: string } }) => module.package?.name === "react")).toBe(true)
    expect(included.some((module: { package?: { name: string } }) => module.package?.name === "zod")).toBe(false)
    expect(bytes).not.toContain(root)
    expect(bytes).not.toContain("!~{")
    expect(graph.chunks.every((chunk: { path: string }) => fs.existsSync(path.join(output, chunk.path)))).toBe(true)
    expect(graph.styles.sources.map((source: { package?: { name: string } }) => source.package?.name))
      .toEqual(expect.arrayContaining(["@xterm/xterm", "tailwindcss", "tw-animate-css"]))
    expect(graph.styles.outputs.length).toBeGreaterThan(0)
    expect(graph.styles.generators.map((source: { name: string }) => source.name)).toContain("@tailwindcss/vite")
  })
  it("fails closed for unknown virtual code or a third-party module without its lock entry", () => {
    expect(() => moduleIdentity("\0unknown-plugin-runtime", root, lock)).toThrow("unknown origin")
    expect(() => moduleIdentity(path.join(root, "node_modules/react/index.js"), root, { packages: {} })).toThrow("no installed lock mapping")
  })
  it("binds transformed template styles and actual singlefile script/CSS bytes", async () => {
    const output = temp()
    await build({ configFile: false, root, logLevel: "silent", plugins: [tailwindcss(), viteSingleFile(), shippedComposition("wizard")],
      build: { outDir: output, emptyOutDir: true, rollupOptions: { input: path.join(root, "test/fixtures/composition-entry.html") } },
    })
    const graph = JSON.parse(fs.readFileSync(path.join(output, "composition-graph.json"), "utf8"))
    const html = htmlContents(fs.readFileSync(path.join(output, "test/fixtures/composition-entry.html"), "utf8"))
    expect(graph.inline_candidates.map((block: { sha256: string }) => block.sha256).sort())
      .toEqual(html.inline.map(block => block.sha256).sort())
    expect(graph.inline_candidates.some((block: { source_block_sha256?: string }) => block.source_block_sha256)).toBe(true)
    expect(html.inline.filter(block => block.kind === "style")).toHaveLength(2)
  })
  it("rejects mismatched installed package versions", () => {
    const stale = structuredClone(lock)
    stale.packages["node_modules/react"].version = "0.0.0"
    expect(() => moduleIdentity(path.join(root, "node_modules/react/index.js"), root, stale)).toThrow("mismatched installed package")
  })
})

function fixture() {
  const directory = temp()
  fs.mkdirSync(path.join(directory, "credential-accept"))
  const output = path.join(directory, "release-integrity/credential-composition.json")
  const source = "frontend/src/lib/zod-csp.ts"
  const graph = { schema: "nyxid.rendered-composition.v1", component: "credential-accept", lock_sha256: hash(fs.readFileSync(path.join(root, "package-lock.json"))), chunks: [
    { path: "entry.js", modules: [{ id: "repo:" + source, source_sha256: hash(fs.readFileSync(path.join(root, "../", source))), rendered_length: 10 }] },
  ], styles: { outputs: [], sources: [], generators: [] } }
  fs.writeFileSync(path.join(directory, "credential-accept/entry.js"), "export const fixture = true;\n")
  fs.writeFileSync(path.join(directory, "credential-accept/composition-graph.json"), JSON.stringify(graph))
  return { directory, output, graph, options: { root: directory, component: "credential-accept", output } }
}
describe("post-transform final-byte seal", () => {
  it("verifies an unchanged tree and rejects changed final code", () => {
    const { directory, options } = fixture()
    seal(options)
    expect(() => seal({ ...options, verify: true })).not.toThrow()
    fs.appendFileSync(path.join(directory, "credential-accept/entry.js"), "// changed\n")
    expect(() => seal({ ...options, verify: true })).toThrow("Stale shipped composition seal")
  })
  it("rejects added unaccounted executable bytes and missing rendered outputs", () => {
    const { directory, options } = fixture()
    fs.writeFileSync(path.join(directory, "credential-accept/extra.js"), "unmapped()")
    expect(() => seal(options)).toThrow("lacks rendered or reviewed static composition")
    fs.unlinkSync(path.join(directory, "credential-accept/extra.js"))
    fs.unlinkSync(path.join(directory, "credential-accept/entry.js"))
    expect(() => seal(options)).toThrow("Rendered output missing")
  })
  it("rejects stale source mappings and symlinked output trees", () => {
    const { directory, graph, options } = fixture()
    graph.chunks[0]!.modules[0]!.source_sha256 = "0".repeat(64)
    fs.writeFileSync(path.join(directory, "credential-accept/composition-graph.json"), JSON.stringify(graph))
    expect(() => seal(options)).toThrow("Stale rendered module source")
    fs.symlinkSync(path.join(directory, "credential-accept/entry.js"), path.join(directory, "link"))
    expect(() => seal(options)).toThrow("Symlink")
  })
  it("requires exact static-font provenance and the shipped license", () => {
    const { directory, options } = fixture()
    const font = path.join(directory, "credential-accept/font.woff2")
    fs.writeFileSync(font, "unmapped font bytes")
    expect(() => seal(options)).toThrow("Unmapped shipped font")
    fs.copyFileSync(path.join(root, "public/nyxbot-inter-latin.woff2"), font)
    expect(() => seal(options)).toThrow("Missing shipped font license")
    fs.copyFileSync(path.join(root, "public/nyxbot-inter-LICENSE.txt"), path.join(directory, "credential-accept/LICENSE.txt"))
    const value = seal(options)
    expect(value.artifacts.find((artifact: { path: string }) => artifact.path.endsWith("woff2")).provenance)
      .toMatchObject({ name: "Inter", origin: "vendored-font", license: "OFL-1.1" })
  })
  it("rejects an unaccounted benign inline script or style in final HTML", () => {
    const { directory, options } = fixture()
    const html = path.join(directory, "credential-accept/credential-accept.html")
    fs.writeFileSync(html, '<!doctype html><script>globalThis.compositionFixture = 1</script>')
    expect(() => seal(options)).toThrow("Unmapped inline script")
    fs.writeFileSync(html, '<!doctype html><style>.composition-fixture{color:blue}</style>')
    expect(() => seal(options)).toThrow("Unmapped inline style")
  })
  it("rejects base elements and attributes mixed-case external resource relations", () => {
    expect(() => htmlContents('<base href="https://unlisted.example/"><script src="entry.js"></script>')).toThrow("base element")
    expect(htmlContents('<link REL="StyleSheet" href="style.css"><link rel="ModulePreload" href="entry.js">').references)
      .toEqual([{ kind: "style", url: "style.css" }, { kind: "script", url: "entry.js" }])
    const { directory, options } = fixture()
    fs.writeFileSync(path.join(directory, "credential-accept/credential-accept.html"), '<link REL="StyleSheet" href="https://unlisted.example/style.css">')
    expect(() => seal(options)).toThrow("Unmapped remote HTML reference")
  })
  it("rejects mislabeled name/purl in an otherwise locked intermediate package mapping", () => {
    const { directory, options, graph } = fixture()
    const identity = moduleIdentity(path.join(root, "node_modules/react/index.js"), root, lock)
    for (const field of ["name", "purl"] as const) {
      const module = { ...structuredClone(identity), rendered_length: 10 }
      module.package![field] = "counterfeit"
      const changed = { ...graph, chunks: [{ path: "entry.js", modules: [module] }] }
      fs.writeFileSync(path.join(directory, "credential-accept/composition-graph.json"), JSON.stringify(changed))
      expect(() => seal(options)).toThrow("mislabeled module package identity")
    }
  })
})
