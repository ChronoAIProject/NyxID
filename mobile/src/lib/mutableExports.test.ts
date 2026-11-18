import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function productionSources(directory: string): Map<string, string> {
  const sources = new Map<string, string>();
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (["__fixtures__", "__tests__"].includes(entry.name)) continue;
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) {
      for (const [file, source] of productionSources(path)) sources.set(file, source);
    } else if (/\.[cm]?[jt]sx?$/.test(entry.name) && !/\.(?:test|spec)\.[cm]?[jt]sx?$/.test(entry.name)) {
      sources.set(path, readFileSync(path, "utf8"));
    }
  }
  return sources;
}

// Resolve aliases and export-star barrels within src, without loading Expo or
// dependencies. Checking the symbol catches local `let` + `export { name }` too.
function mutableExports(sources: Map<string, string>): string[] {
  const options: ts.CompilerOptions = { noLib: true, allowJs: true, jsx: ts.JsxEmit.Preserve };
  const host = ts.createCompilerHost(options);
  host.getSourceFile = (file, version) => {
    const source = sources.get(file);
    return source === undefined ? undefined : ts.createSourceFile(file, source, version, true);
  };
  host.resolveModuleNames = (names, containingFile) => names.map((name) => {
    const base = resolve(dirname(containingFile), name);
    const file = [base, ...[".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"].flatMap((ext) => [base + ext, resolve(base, `index${ext}`)])]
      .find((candidate) => sources.has(candidate));
    return file ? { resolvedFileName: file } : undefined;
  });
  const program = ts.createProgram([...sources.keys()], options, host);
  const checker = program.getTypeChecker();
  const violations: string[] = [];
  for (const file of program.getSourceFiles()) {
    const module = checker.getSymbolAtLocation(file);
    if (!module) continue;
    for (const exported of checker.getExportsOfModule(module)) {
      const symbol = exported.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(exported) : exported;
      for (const declaration of symbol.declarations ?? []) {
        let node: ts.Node | undefined = declaration;
        while (node && !ts.isVariableDeclarationList(node)) node = node.parent;
        if (node && !(node.flags & ts.NodeFlags.Const)) {
          violations.push(`${relative(root, file.fileName)}: ${exported.name}`);
          break;
        }
      }
    }
  }
  return violations.sort();
}

test("production modules expose functions or constants, never mutable bindings or their re-exports", () => {
  assert.deepEqual(mutableExports(productionSources(root)), []);
});

test("the source guard catches direct, local, aliased and star mutable exports", () => {
  const sources = new Map(Object.entries({
    "state.ts": "export let value = 0; export var other = 1; export const fixed = 2; export function read() { return value; }",
    "local.ts": "let local = 0; const fixed = 1; export { local, fixed };",
    "alias.ts": "export { value as selected, fixed, read } from './state';",
    "star.ts": "export * from './alias';",
    "imported.ts": "import { value } from './state'; export { value as copied };",
    "destructured.ts": "export let { nested } = { nested: 0 };",
  }).map(([name, source]) => [resolve(root, name), source]));
  assert.deepEqual(mutableExports(sources), [
    "alias.ts: selected", "destructured.ts: nested", "imported.ts: copied",
    "local.ts: local", "star.ts: selected", "state.ts: other", "state.ts: value",
  ]);
});
