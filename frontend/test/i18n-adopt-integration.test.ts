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

  it("adopts a conflicted cherry-pick of a legacy commit", () => {
    const f = fixture();
    f.git("checkout", "split");
    expect(spawnSync("git", ["cherry-pick", f.branch], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
    const adopted = f.adopt();
    expect(adopted.status, adopted.stderr).toBe(0);
    expect(adopted.stdout).toContain(`i18n: adopted CHERRY_PICK_HEAD (base CHERRY_PICK_HEAD^) onto HEAD`);
    f.verify();
  });

  it("explains git am instead of failing on a missing REBASE_HEAD", () => {
    const f = fixture();
    const patch = path.join(f.repo, "legacy.patch");
    fs.writeFileSync(patch, f.git("format-patch", "-1", "--stdout", f.branch));
    f.git("checkout", "split");
    expect(spawnSync("git", ["am", "-3", patch], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
    const result = f.adopt();
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("git am is in progress; a mailbox patch has no commit to read the old catalogs from");
    expect(result.stderr).toContain("usage: npm run adopt:i18n -- --branch <ref> --base <ref|empty> --onto <ref>");
    expect(result.stderr).not.toContain("bad revision");
  });

  it("asks for explicit refs when a rebase has no usable REBASE_HEAD", () => {
    const f = fixture();
    expect(spawnSync("git", ["rebase", "split"], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
    fs.rmSync(path.join(f.repo, ".git/REBASE_HEAD"));
    const result = f.adopt();
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("a rebase is in progress but REBASE_HEAD does not resolve");
    expect(result.stderr).toContain("usage: npm run adopt:i18n -- --branch <ref> --base <ref|empty> --onto <ref>");
  });

  it("reports when no merge, rebase or cherry-pick is in progress", () => {
    const f = fixture();
    const result = f.adopt();
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("no merge, rebase or cherry-pick detected; provide --branch, --base and --onto.");
  });

  /** Merge split into legacy, keep the legacy monoliths (and legacy families), and commit. */
  const commitMergeKeepingMonoliths = (f: ReturnType<typeof fixture>) => {
    f.git("checkout", "legacy");
    // As in the real repository, the monoliths are not renamed into an area (the fixture is small enough to trip rename detection).
    spawnSync("git", ["merge", "--no-commit", "-X", "no-renames", "split"], { cwd: f.repo, encoding: "utf8" });
    const conflicted = f.git("diff", "--name-only", "--diff-filter=U").split("\n").filter(Boolean);
    f.git("checkout", "--ours", "--", ...conflicted);
    f.git("add", "-A", "frontend/src");
    f.git("commit", "-m", "merge that kept the monoliths");
    for (const language of LANGUAGES) expect(fs.existsSync(path.join(f.locales, `${language}.json`))).toBe(true);
    expect(fs.existsSync(path.join(f.locales, "app"))).toBe(true);
    return f.git("rev-parse", "HEAD");
  };

  it("uses a mixed branch's monoliths so its legacy edits land and nothing is lost", () => {
    const f = fixture();
    const mixed = commitMergeKeepingMonoliths(f);
    const adopted = f.adopt("--branch", mixed, "--base", f.base, "--onto", f.onto);
    expect(adopted.status, adopted.stderr).toBe(0);
    f.verify();
  });

  it("refuses a mixed branch whose split areas hold values found nowhere else, and writes nothing", () => {
    const f = fixture();
    commitMergeKeepingMonoliths(f);
    const file = path.join(f.locales, "app/unattributed/zh-TW.json");
    f.catalog(file, { ...JSON.parse(fs.readFileSync(file, "utf8")), "Area only": "only here" });
    for (const language of ["en", "zh-CN"]) {
      const other = path.join(f.locales, `app/unattributed/${language}.json`);
      f.catalog(other, { ...JSON.parse(fs.readFileSync(other, "utf8")), "Area only": `only ${language}` });
    }
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "edit the areas too");
    const before = f.git("status", "--porcelain");
    const result = f.adopt("--branch", "HEAD", "--base", f.base, "--onto", f.onto);
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("No files written. HEAD (--branch) holds both the legacy monoliths and the split areas, and 3 value(s) exist only in its split areas");
    expect(result.stderr).toContain('--branch HEAD^1 --base "$(git merge-base HEAD^1 HEAD^2)" --onto HEAD^2');
    expect(f.git("status", "--porcelain")).toBe(before);
    for (const language of LANGUAGES) expect(fs.existsSync(path.join(f.locales, `${language}.json`))).toBe(true);
  });

  it("refuses a snapshot with only some legacy monoliths", () => {
    const f = fixture();
    f.git("checkout", "legacy");
    f.git("rm", "-q", "frontend/src/i18n/locales/zh-TW.json");
    f.git("commit", "-m", "partial monoliths");
    const result = f.adopt("--branch", "HEAD", "--base", f.base, "--onto", f.onto);
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("No files written. HEAD (--branch) has only some legacy monoliths (en.json, zh-CN.json)");
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
