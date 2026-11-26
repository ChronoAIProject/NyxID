// @vitest-environment node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import ts from "typescript";
import {
  catalogTools, gitRepository, legacyImportReplacement, moduleSpecifierLister, rewriteLegacyImportsInTree, scanUsage,
} from "../../shared/i18n/catalog-core.mjs";

const corePath = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../shared/i18n/catalog-core.mjs");
const temporaryDirs: string[] = [];
afterEach(() => {
  for (const dir of temporaryDirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});
const temporaryDir = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-i18n-core-"));
  temporaryDirs.push(dir);
  return dir;
};

describe("shared i18n catalog core", () => {
  it("depends only on Node built-ins so each package supplies its own parser", () => {
    const imports = [...fs.readFileSync(corePath, "utf8").matchAll(/^import .* from "([^"]+)";$/gm)].map((match) => match[1]);
    expect(imports.every((specifier) => specifier.startsWith("node:"))).toBe(true);
  });

  it("binds analysis, consolidation and messages to the caller's languages and label", () => {
    const tools = catalogTools(["zh-CN", "zh-TW"], { label: "mobile" });
    expect(tools.emptyLanguages()).toEqual({ "zh-CN": {}, "zh-TW": {} });
    const portions = {
      account: { "zh-CN": { Save: "保存", Only: "仅" }, "zh-TW": { Save: "儲存" } },
      auth: { "zh-CN": { Save: "保存" }, "zh-TW": { Save: "儲存" } },
    };
    expect(tools.analyzePortions(portions).issues).toEqual(["mobile/account: key parity differs across zh-CN/zh-TW"]);
    delete (portions.account["zh-CN"] as Record<string, string>).Only;
    const next = tools.consolidateDuplicates(portions);
    expect(next.shared).toEqual({ "zh-CN": { Save: "保存" }, "zh-TW": { Save: "儲存" } });
    expect(next.account).toEqual({ "zh-CN": {}, "zh-TW": {} });
    portions.auth["zh-TW"].Save = "存";
    expect(() => tools.consolidateDuplicates(portions)).toThrow("mobile duplicate key has differing values: Save (account, auth)");
  });

  it("places by the caller's predicate, area map and literal parser", () => {
    const scan = {
      isSource: (file: string) => !file.endsWith(".test.ts"),
      areaForPath: (file: string) => file.split("/")[2] ?? "shared",
      literals: (_file: string, source: string) => [...source.matchAll(/"([^"]+)"/g)].map((match) => match[1]),
    };
    const usage = scanUsage([
      ["src/features/account/a.ts", '"Save" "Both"'],
      ["src/features/auth/b.ts", '"Both"'],
      ["src/features/auth/b.test.ts", '"Save" "Test only"'],
    ], ["Save", "Both", "Test only"], scan);
    expect(Object.fromEntries(usage)).toEqual({ Save: "account", Both: "shared", "Test only": "unattributed" });
    const tools = catalogTools(["zh-CN"], { label: "mobile" });
    const placed = tools.placeCatalogs({ "zh-CN": { Save: "保存", Both: "都" } }, [["src/features/account/a.ts", '"Save" "Both"']], { auth: { "zh-CN": { Both: "都" } } }, scan);
    expect(placed).toEqual({ auth: { "zh-CN": { Both: "都" } }, account: { "zh-CN": { Save: "保存" } } });
  });

  it("merges named catalog deltas language-major and reports every conflict", () => {
    const tools = catalogTools(["en", "zh-CN"], { label: "app" });
    const snapshot = (app: string, extra: string) => ({ app: { en: { Key: app }, "zh-CN": { Key: app } }, extra: { en: { Key: extra }, "zh-CN": { Key: extra } } });
    const clean = tools.mergeDeltas(snapshot("a", "x"), snapshot("b", "x"), snapshot("a", "y"), ["app", "extra"], false);
    expect(clean.conflicts).toEqual([]);
    expect(clean.result.app.en).toEqual({ Key: "b" });
    expect(clean.result.extra["zh-CN"]).toEqual({ Key: "y" });
    const conflicted = tools.mergeDeltas(snapshot("a", "x"), snapshot("b", "z"), snapshot("c", "y"), ["app", "extra"], false);
    expect(conflicted.conflicts).toEqual([
      "app/en: changed key diverged: Key", "extra/en: changed key diverged: Key",
      "app/zh-CN: changed key diverged: Key", "extra/zh-CN: changed key diverged: Key",
    ]);
    const empty = { app: tools.emptyLanguages(), extra: tools.emptyLanguages() };
    const adopted = tools.mergeDeltas(empty, snapshot("b", "x"), snapshot("c", "y"), ["app", "extra"], true);
    expect(adopted.conflicts).toEqual([]);
    expect(adopted.result.app.en).toEqual({ Key: "b" });
  });

  it("writes, loads and prunes area directories", () => {
    const dir = temporaryDir();
    const tools = catalogTools(["zh-CN", "zh-TW"], { label: "mobile" });
    const one = { "zh-CN": { k: "1" }, "zh-TW": { k: "1" } };
    tools.writePortions(dir, { b: { "zh-CN": { z: "1", A: "2" }, "zh-TW": { z: "1", A: "2" } }, a: one, empty: { "zh-CN": {}, "zh-TW": {} } });
    expect(fs.readFileSync(path.join(dir, "b/zh-CN.json"), "utf8")).toBe('{\n  "A": "2",\n  "z": "1"\n}\n');
    expect(Object.keys(tools.loadPortions(dir))).toEqual(["a", "b"]);
    tools.writePortions(dir, { b: tools.loadPortions(dir).b, a: { "zh-CN": {}, "zh-TW": {} } });
    expect(fs.existsSync(path.join(dir, "a"))).toBe(false);
  });

  it("reads one snapshot through a single cat-file batch, including multibyte and spaced paths", () => {
    const repo = temporaryDir();
    const git = (...args: string[]) => execFileSync("git", args, { cwd: repo, encoding: "utf8" }).trim();
    git("init", "-q", "-b", "main");
    git("config", "user.name", "core"); git("config", "user.email", "core@example.test"); git("config", "commit.gpgsign", "false");
    const pkg = path.join(repo, "pkg");
    fs.mkdirSync(path.join(pkg, "src/a b"), { recursive: true });
    fs.writeFileSync(path.join(pkg, "src/a b/zh-TW.json"), '{"設定":"設定"}\n');
    fs.writeFileSync(path.join(pkg, "src/empty.txt"), "");
    fs.writeFileSync(path.join(pkg, "src/skip.ts"), "skip");
    git("add", "."); git("commit", "-q", "-m", "snapshot");
    const repository = gitRepository(pkg);
    expect(repository.prefix).toBe("pkg");
    const files = repository.snapshotFiles("HEAD", "src", (file: string) => !file.endsWith(".ts"));
    expect([...files]).toEqual([["src/a b/zh-TW.json", '{"設定":"設定"}\n'], ["src/empty.txt", ""]]);
    expect(repository.snapshotFiles("empty", "src", () => true).size).toBe(0);
    expect(() => repository.resolveAdoptRefs(["--branch", "HEAD"], "usage")).toThrow("must be supplied together");
    expect(() => repository.resolveAdoptRefs(["--bogus", "HEAD"], "usage text")).toThrow("usage text");
    expect(() => repository.resolveAdoptRefs([], "usage")).toThrow("no merge, rebase or cherry-pick detected");
    expect(repository.resolveAdoptRefs(["--branch", "a", "--base", "empty", "--onto", "b"], "usage")).toEqual({ branch: "a", base: "empty", onto: "b" });
  });

  it("rewrites legacy imports for any package map, keeping relative prefixes, and logs each rewrite", () => {
    const legacy = { "src/i18n/locales/zh-CN.json": "src/i18n/locales/zh-CN", "src/i18n/locales/zh-TW.json": "src/i18n/locales/zh-TW" };
    expect(legacyImportReplacement("src/i18n/translate.ts", "./locales/zh-TW.json", legacy, [])).toBe("./locales/zh-TW");
    expect(legacyImportReplacement("src/features/a/b.test.ts", "../../i18n/locales/zh-CN.json", legacy, [])).toBe("../../i18n/locales/zh-CN");
    expect(legacyImportReplacement("src/features/a/b.test.ts", "@/i18n/locales/zh-CN.json", legacy, [])).toBeNull();
    const root = temporaryDir();
    fs.mkdirSync(path.join(root, "src/i18n"), { recursive: true });
    fs.writeFileSync(path.join(root, "src/i18n/x.test.tsx"), 'import zh from "./locales/zh-CN.json";\njest.mock("./locales/zh-TW.json");\nimport other from "./other.json";\n');
    fs.writeFileSync(path.join(root, "src/i18n/plain.ts"), "export const x = 1;\n");
    const logs: string[] = [];
    const run = () => rewriteLegacyImportsInTree({ root, dir: "src", specifiers: moduleSpecifierLister(ts), legacy, aliases: [], typecheck: "pnpm typecheck", log: (line: string) => logs.push(line) });
    expect(run()).toEqual(["src/i18n/x.test.tsx"]);
    expect(fs.readFileSync(path.join(root, "src/i18n/x.test.tsx"), "utf8")).toBe('import zh from "./locales/zh-CN";\njest.mock("./locales/zh-TW");\nimport other from "./other.json";\n');
    expect(logs).toEqual([
      "i18n: rewrote src/i18n/x.test.tsx:1 ./locales/zh-CN.json -> ./locales/zh-CN",
      "i18n: rewrote src/i18n/x.test.tsx:2 ./locales/zh-TW.json -> ./locales/zh-TW",
      "i18n: stage the rewritten imports too: git add src/i18n/x.test.tsx",
      "i18n: run pnpm typecheck to type-check the result.",
    ]);
    logs.length = 0;
    expect(run()).toEqual([]);
    expect(logs).toEqual(["i18n: run pnpm typecheck to type-check the result."]);
  });
});
