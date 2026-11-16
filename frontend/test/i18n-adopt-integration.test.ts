// @vitest-environment node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import {
  LANGUAGES, FAMILIES, analyzePortions, canonicalText, loadPortions,
} from "../scripts/lib/i18n-catalogs.mjs";

const frontend = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const temporaryRepos: string[] = [];
afterEach(() => {
  for (const repo of temporaryRepos.splice(0)) fs.rmSync(repo, { recursive: true, force: true });
});

function fixture() {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-i18n-adopt-"));
  temporaryRepos.push(repo);
  const root = path.join(repo, "frontend");
  const locales = path.join(root, "src/i18n/locales");
  const git = (...args: string[]) => execFileSync("git", args, { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  const write = (file: string, text: string) => { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text); };
  const catalog = (file: string, value: object) => write(file, canonicalText(value));
  fs.mkdirSync(path.join(root, "scripts/lib"), { recursive: true });
  for (const file of ["adopt-legacy-i18n.mjs", "lib/i18n-catalogs.mjs", "lib/i18n-portions.mjs"]) fs.copyFileSync(path.join(frontend, "scripts", file), path.join(root, "scripts", file));
  write(path.join(root, "package.json"), '{"type":"module"}\n');
  fs.symlinkSync(path.join(frontend, "node_modules"), path.join(root, "node_modules"));
  git("init", "-b", "legacy");
  git("config", "user.name", "i18n rehearsal");
  git("config", "user.email", "i18n@example.test");
  git("config", "core.hooksPath", "/dev/null");
  git("config", "commit.gpgsign", "false");
  for (const language of LANGUAGES) {
    catalog(path.join(locales, `${language}.json`), { "Change me": `old ${language}`, "Remove me": `removed ${language}`, Kept: `kept ${language}` });
    for (const family of FAMILIES) catalog(path.join(locales, `${family}-${language}.json`), { Kept: `kept ${language}`, "Change me": `old ${language}`, "Remove me": `removed ${language}` });
  }
  catalog(path.join(locales, "en-overrides.json"), { "Change me": "old en" });
  write(path.join(root, "src/pages/billing.ts"), 'const copy = "Kept";\n');
  write(path.join(root, "tsconfig.json"), `${JSON.stringify({
    compilerOptions: {
      target: "ES2022", module: "ESNext", moduleResolution: "bundler", resolveJsonModule: true, types: ["vite/client"],
      strict: true, noUncheckedIndexedAccess: true, noEmit: true, skipLibCheck: true, baseUrl: ".", paths: { "@/*": ["./src/*"] },
    },
    include: ["src"],
  }, null, 2)}\n`);
  git("add", "frontend/scripts", "frontend/package.json", "frontend/tsconfig.json", "frontend/src");
  git("commit", "-m", "base old layout");
  const base = git("rev-parse", "HEAD");
  const adopt = (...args: string[]) => spawnSync(process.execPath, [path.join(root, "scripts/adopt-legacy-i18n.mjs"), ...args], { cwd: root, encoding: "utf8" });
  git("checkout", "-b", "split");
  const initial = adopt("--branch", base, "--base", "empty", "--onto", base);
  expect(initial.status, initial.stderr).toBe(0);
  expect(fs.existsSync(path.join(locales, "en-overrides.json"))).toBe(false);
  expect(fs.existsSync(path.join(locales, "app/en-overrides.json"))).toBe(true);
  for (const barrel of ["en-full.ts", "zh-CN.ts", "zh-TW.ts"]) fs.copyFileSync(path.join(frontend, "src/i18n/locales/app", barrel), path.join(locales, "app", barrel));
  git("add", "frontend/src"); git("commit", "-m", "split layout");
  const onto = git("rev-parse", "HEAD");
  git("checkout", "legacy");
  for (const language of LANGUAGES) {
    catalog(path.join(locales, `${language}.json`), { "Change me": `new ${language}`, Kept: `kept ${language}`, "Added key": `added ${language}` });
    for (const family of FAMILIES) catalog(path.join(locales, `${family}-${language}.json`), { Kept: `kept ${language}`, "Change me": `new ${language}`, "Added key": `added ${language}` });
  }
  write(path.join(root, "src/pages/billing.ts"), 'const copy = ["Kept", "Added key"];\n');
  // An in-flight test that imports a pre-split catalog path.
  write(path.join(root, "src/pages/billing-copy.test.ts"), 'import zhCN from "@/i18n/locales/zh-CN.json";\n\nexport const kept: string = zhCN["Kept"] ?? "Kept";\n');
  git("add", "frontend/src"); git("commit", "-m", "legacy catalog delta");
  const branch = git("rev-parse", "HEAD");
  const verify = () => {
    const { merged, issues } = analyzePortions(loadPortions(locales));
    expect(issues).toEqual([]);
    for (const language of LANGUAGES) {
      const expected = { "Change me": `new ${language}`, Kept: `kept ${language}`, "Added key": `added ${language}` };
      expect(merged[language]).toEqual(expected);
      expect(JSON.parse(fs.readFileSync(path.join(locales, "app/billing", `${language}.json`), "utf8"))["Added key"]).toBe(`added ${language}`);
      for (const family of FAMILIES) expect(JSON.parse(fs.readFileSync(path.join(locales, `${family}-${language}.json`), "utf8"))).toEqual(expected);
      expect(fs.existsSync(path.join(locales, `${language}.json`))).toBe(false);
    }
    expect(fs.existsSync(path.join(locales, "en-overrides.json"))).toBe(false);
  };
  return { repo, root, locales, git, adopt, base, branch, onto, verify, catalog };
}

describe("adopt legacy catalogs using real Git snapshots", () => {
  it.each(["merge", "rebase"])("resolves a real %s, stages every catalog family and rewrites legacy imports so they type-check", (mode) => {
    const f = fixture();
    const args = mode === "merge" ? ["merge", "--no-commit", "split"] : ["rebase", "split"];
    const merge = spawnSync("git", args, { cwd: f.repo, encoding: "utf8" });
    expect(merge.status).not.toBe(0);
    const adopted = f.adopt();
    expect(adopted.status, adopted.stderr).toBe(0);
    f.verify();
    expect(adopted.stdout).toContain("i18n: rewrote src/pages/billing-copy.test.ts:1 @/i18n/locales/zh-CN.json -> @/i18n/locales/app/zh-CN");
    expect(adopted.stdout).toContain("git add src/pages/billing-copy.test.ts");
    expect(adopted.stdout).toContain("run npx tsc -b");
    expect(fs.readFileSync(path.join(f.root, "src/pages/billing-copy.test.ts"), "utf8")).toBe('import zhCN from "@/i18n/locales/app/zh-CN";\n\nexport const kept: string = zhCN["Kept"] ?? "Kept";\n');
    const typecheck = spawnSync(process.execPath, [path.join(f.root, "node_modules/typescript/bin/tsc"), "-p", "tsconfig.json"], { cwd: f.root, encoding: "utf8" });
    expect(typecheck.status, typecheck.stdout).toBe(0);
    f.git("add", "-A", "frontend/src/i18n/locales", "frontend/src/pages/billing-copy.test.ts");
    expect(f.git("diff", "--name-only", "--diff-filter=U")).toBe("");
    if (mode === "rebase") f.git("-c", "core.editor=true", "rebase", "--continue");
    else f.git("commit", "-m", "resolved merge");
    f.verify();
  });

  it("rewrites legacy imports outside a merge or rebase with --imports-only", () => {
    const f = fixture();
    f.git("checkout", "split");
    const file = path.join(f.root, "src/pages/late.test.ts");
    fs.writeFileSync(file, 'const en = await import("../i18n/locales/en.json");\n');
    const result = f.adopt("--imports-only");
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toContain("i18n: rewrote src/pages/late.test.ts:1 ../i18n/locales/en.json -> ../i18n/locales/app/en-full");
    expect(fs.readFileSync(file, "utf8")).toBe('const en = await import("../i18n/locales/app/en-full");\n');
    expect(f.git("status", "--porcelain", "--", "frontend/src")).toBe("?? frontend/src/pages/late.test.ts");
  });

  it("rejects divergence in a later language without writing any files", () => {
    const f = fixture();
    f.git("checkout", "split");
    const file = path.join(f.locales, "app/unattributed/zh-TW.json");
    const data = JSON.parse(fs.readFileSync(file, "utf8"));
    data["Change me"] = "onto conflict";
    f.catalog(file, data);
    f.git("add", "frontend/src"); f.git("commit", "-m", "divergent onto value");
    const before = f.git("status", "--porcelain");
    const result = f.adopt("--branch", f.branch, "--base", f.base, "--onto", "HEAD");
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("No files written");
    expect(result.stderr).toContain("app/zh-TW: changed key diverged: Change me");
    expect(f.git("status", "--porcelain")).toBe(before);
  });
});
