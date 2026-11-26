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
  fs.mkdirSync(path.join(repo, "shared/i18n"), { recursive: true });
  fs.copyFileSync(path.join(frontend, "../shared/i18n/catalog-core.mjs"), path.join(repo, "shared/i18n/catalog-core.mjs"));
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
  git("add", "shared", "frontend/scripts", "frontend/package.json", "frontend/tsconfig.json", "frontend/src");
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
    fs.writeFileSync(file, 'const en = await import("../i18n/locales/en.json");\nimport zh from "../i18n/locales/zh-CN.json" with { type: "json" };\n');
    const result = f.adopt("--imports-only");
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toContain("i18n: rewrote src/pages/late.test.ts:1 ../i18n/locales/en.json -> ../i18n/locales/app/en-full\n");
    expect(result.stdout).toContain("i18n: rewrote src/pages/late.test.ts:2 ../i18n/locales/zh-CN.json -> ../i18n/locales/app/zh-CN (removed its JSON import attributes)\n");
    expect(fs.readFileSync(file, "utf8")).toBe('const en = await import("../i18n/locales/app/en-full");\nimport zh from "../i18n/locales/app/zh-CN";\n');
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

  /** After the merge that kept the monoliths (M), main moves on: it changes "Kept" and adds "Main new". */
  const mainMovesAfterMixedMerge = (f: ReturnType<typeof fixture>, afterMerge = () => {}) => {
    const mixed = commitMergeKeepingMonoliths(f);
    afterMerge();
    f.git("checkout", "split");
    for (const language of LANGUAGES) {
      const file = path.join(f.locales, "app/billing", `${language}.json`);
      f.catalog(file, { ...JSON.parse(fs.readFileSync(file, "utf8")), Kept: `kept v2 ${language}`, "Main new": `main ${language}` });
    }
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "main moves on");
    const moved = f.git("rev-parse", "HEAD");
    f.git("checkout", "legacy");
    return { mixed, moved };
  };
  /** Run the guidance command adopt printed, through this fixture's script. */
  const runGuidance = (f: ReturnType<typeof fixture>, stderr: string) => {
    const command = stderr.split("\n").find((line) => line.startsWith("  npm run adopt:i18n -- --branch"));
    expect(command, stderr).toBeDefined();
    return spawnSync("sh", ["-c", command!.trim().replace("npm run adopt:i18n --", `"${process.execPath}" scripts/adopt-legacy-i18n.mjs`)], { cwd: f.root, encoding: "utf8" });
  };
  /** The branch's legacy delta on top of main's later changes, with nothing of main reverted. */
  const expectNothingReverted = (f: ReturnType<typeof fixture>, extra: (language: string) => object = () => ({})) => {
    const { merged, issues } = analyzePortions(loadPortions(f.locales));
    expect(issues).toEqual([]);
    for (const language of LANGUAGES) {
      expect(merged[language]).toEqual({ "Change me": `new ${language}`, Kept: `kept v2 ${language}`, "Added key": `added ${language}`, "Main new": `main ${language}`, ...extra(language) });
      expect(fs.existsSync(path.join(f.locales, `${language}.json`))).toBe(false);
    }
  };

  it("refuses a mixed branch against a split-only --base, and its guidance adopts without reverting main", () => {
    const f = fixture();
    const { mixed, moved } = mainMovesAfterMixedMerge(f);
    const before = f.git("status", "--porcelain");
    const refused = f.adopt("--branch", mixed, "--base", f.onto, "--onto", moved);
    expect(refused.status).not.toBe(0);
    expect(refused.stderr).toContain(`No files written. ${mixed} (--branch) keeps the legacy monoliths beside split areas it inherited from main, so its own edits live in the monoliths (and any later edits to those areas). ${f.onto} (--base) has only the split areas`);
    expect(refused.stderr).toContain(`  npm run adopt:i18n -- --branch ${mixed} --base ${f.base} --onto ${moved}`);
    expect(refused.stderr).not.toContain("^1");
    expect(f.git("status", "--porcelain")).toBe(before);
    const guided = runGuidance(f, refused.stderr);
    expect(guided.status, guided.stderr).toBe(0);
    expectNothingReverted(f);
  });

  it("refuses the auto-detected merge of a branch that kept the monoliths, and its guidance completes the merge", () => {
    const f = fixture();
    mainMovesAfterMixedMerge(f);
    spawnSync("git", ["merge", "--no-commit", "split"], { cwd: f.repo, encoding: "utf8" });
    expect(fs.existsSync(path.join(f.repo, ".git/MERGE_HEAD"))).toBe(true);
    const before = f.git("status", "--porcelain");
    const refused = f.adopt();
    expect(refused.status).not.toBe(0);
    expect(refused.stderr).toContain("HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main");
    expect(refused.stderr).toContain(`  npm run adopt:i18n -- --branch HEAD --base ${f.base} --onto MERGE_HEAD`);
    expect(f.git("status", "--porcelain")).toBe(before);
    const guided = runGuidance(f, refused.stderr);
    expect(guided.status, guided.stderr).toBe(0);
    expectNothingReverted(f);
    f.git("add", "-A", "frontend/src/i18n/locales");
    f.git("commit", "-m", "second merge");
    expectNothingReverted(f);
  });

  it("adopts a branch that edited its monoliths after the merge that kept them, following the printed guidance", () => {
    const f = fixture();
    // Main changes "Kept" before the branch merges it, and again afterwards.
    f.git("checkout", "split");
    for (const language of LANGUAGES) {
      const file = path.join(f.locales, "app/billing", `${language}.json`);
      f.catalog(file, { ...JSON.parse(fs.readFileSync(file, "utf8")), Kept: `kept v1 ${language}` });
    }
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "main changes Kept");
    mainMovesAfterMixedMerge(f, () => {
      // X: after the merge that kept the monoliths, the branch edits them again.
      for (const language of LANGUAGES) {
        const file = path.join(f.locales, `${language}.json`);
        f.catalog(file, { ...JSON.parse(fs.readFileSync(file, "utf8")), "X key": `x ${language}` });
      }
      f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "X edits the monoliths");
    });
    expect(f.git("rev-list", "--parents", "-n", "1", "HEAD").split(" ")).toHaveLength(2);
    // The branch merges main again; adopt, then follow exactly what it prints.
    spawnSync("git", ["merge", "--no-commit", "split"], { cwd: f.repo, encoding: "utf8" });
    const before = f.git("status", "--porcelain");
    const refused = f.adopt();
    expect(refused.status).not.toBe(0);
    expect(refused.stderr).toContain("HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main");
    expect(refused.stderr).not.toContain("^1");
    expect(refused.stderr).toContain(`  npm run adopt:i18n -- --branch HEAD --base ${f.base} --onto MERGE_HEAD`);
    expect(f.git("status", "--porcelain")).toBe(before);
    const guided = runGuidance(f, refused.stderr);
    expect(guided.status, guided.stderr).toBe(0);
    expectNothingReverted(f, (language) => ({ "X key": `x ${language}` }));
    f.git("add", "-A", "frontend/src/i18n/locales");
    f.git("commit", "-m", "merge main again");
    expectNothingReverted(f, (language) => ({ "X key": `x ${language}` }));
  });

  it("prints only generic fork guidance when the split reached onto through a merge commit", () => {
    const f = fixture();
    // Main moves past the split branch's fork, a branch forks from there, then main merges the split with a merge commit.
    f.git("checkout", "-b", "main2", f.base);
    const common = path.join(f.locales, "common-en.json");
    f.catalog(common, { ...JSON.parse(fs.readFileSync(common, "utf8")), "Main moved": "Main moved" });
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "main moves past the fork");
    const laterFork = f.git("rev-parse", "HEAD");
    f.git("checkout", "-b", "legacy2");
    for (const language of LANGUAGES) {
      const monolith = path.join(f.locales, `${language}.json`);
      f.catalog(monolith, { ...JSON.parse(fs.readFileSync(monolith, "utf8")), "Branch key": `branch ${language}` });
    }
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "legacy branch from the later fork");
    f.git("checkout", "main2");
    f.git("merge", "--no-ff", "-m", "merge the split", "split");
    // The branch merges that main keeping its monoliths, so the split commit is on its history.
    f.git("checkout", "legacy2");
    spawnSync("git", ["merge", "--no-commit", "-X", "no-renames", "main2"], { cwd: f.repo, encoding: "utf8" });
    const conflicted = f.git("diff", "--name-only", "--diff-filter=U").split("\n").filter(Boolean);
    if (conflicted.length) f.git("checkout", "--ours", "--", ...conflicted);
    f.git("checkout", "legacy2@{0}", "--", ...LANGUAGES.map((language) => `frontend/src/i18n/locales/${language}.json`));
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "merge main2, keeping the monoliths");
    const mixed = f.git("rev-parse", "HEAD");
    const result = f.adopt("--branch", mixed, "--base", f.onto, "--onto", "main2");
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("Pass the branch's legacy fork (the newest pre-split commit it is based on) as --base:");
    expect(result.stderr).toContain(`  npm run adopt:i18n -- --branch ${mixed} --base <legacy-fork> --onto main2`);
    // The split branch's fork would be the wrong base for a branch that forked from a later main.
    expect(result.stderr).not.toContain(`--base ${f.base}`);
    expect(result.stderr).not.toContain(`--base ${laterFork}`);
  });


  it("refuses a split-only branch against a legacy-only base", () => {
    const f = fixture();
    const result = f.adopt("--branch", f.onto, "--base", f.base, "--onto", f.onto);
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain(`No files written. ${f.onto} (--branch) has no legacy monoliths, so it is not a branch from before the split and there is nothing to adopt`);
  });

  it("adopts edits a mixed branch made to the split areas it inherited", () => {
    const f = fixture();
    commitMergeKeepingMonoliths(f);
    for (const language of LANGUAGES) {
      const file = path.join(f.locales, `app/unattributed/${language}.json`);
      f.catalog(file, { ...JSON.parse(fs.readFileSync(file, "utf8")), "Area only": `only ${language}` });
    }
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "edit the areas too");
    const result = f.adopt("--branch", "HEAD", "--base", f.base, "--onto", f.onto);
    expect(result.status, result.stderr).toBe(0);
    const { merged } = analyzePortions(loadPortions(f.locales));
    for (const language of LANGUAGES) {
      expect(merged[language]).toEqual({ "Change me": `new ${language}`, Kept: `kept ${language}`, "Added key": `added ${language}`, "Area only": `only ${language}` });
      expect(fs.existsSync(path.join(f.locales, `${language}.json`))).toBe(false);
    }
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

  /** Apply `edit` to every monolith (or to one language's) and commit. */
  const editMonoliths = (f: ReturnType<typeof fixture>, message: string, edit: (catalog: Record<string, string>, language: string) => void, only?: string) => {
    for (const language of only ? [only] : LANGUAGES) {
      const file = path.join(f.locales, `${language}.json`);
      const catalog = JSON.parse(fs.readFileSync(file, "utf8"));
      edit(catalog, language);
      f.catalog(file, catalog);
    }
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", message);
    return f.git("rev-parse", "HEAD");
  };
  /** A legacy branch merge commit: a side branch from the legacy tip edits the monoliths, legacy merges it with --no-ff. */
  const legacyMergeCommit = (f: ReturnType<typeof fixture>) => {
    f.git("checkout", "-q", "-b", "side", "legacy");
    editMonoliths(f, "side edits the monoliths", (catalog, language) => { catalog["Side key"] = `side ${language}`; });
    f.git("checkout", "-q", "legacy");
    fs.writeFileSync(path.join(f.root, "src/pages/legacy-only.ts"), "export const x = 1;\n");
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "legacy moves on");
    f.git("merge", "--no-ff", "-m", "merge side", "side");
    return f.git("rev-parse", "HEAD");
  };
  /** Assert a refusal that writes nothing and prints no command to run instead. */
  const expectRefused = (f: ReturnType<typeof fixture>, result: ReturnType<typeof spawnSync>, before: string, message: string | RegExp) => {
    expect(result.status).not.toBe(0);
    expect(result.stderr).toContain("No files written.");
    if (typeof message === "string") expect(result.stderr).toContain(message);
    else expect(result.stderr).toMatch(message);
    expect(result.stderr).not.toContain("  npm run adopt:i18n -- --branch");
    expect(f.git("status", "--porcelain")).toBe(before);
  };
  /** Split `ref` the way main did: a new branch with the adopted split layout committed. */
  const splitAt = (f: ReturnType<typeof fixture>, name: string, ref: string) => {
    f.git("checkout", "-q", "-b", name, ref);
    const result = f.adopt("--branch", "HEAD", "--base", "empty", "--onto", "HEAD");
    expect(result.status, result.stderr).toBe(0);
    for (const barrel of ["en-full.ts", "zh-CN.ts", "zh-TW.ts"]) fs.copyFileSync(path.join(frontend, "src/i18n/locales/app", barrel), path.join(f.locales, "app", barrel));
    f.git("add", "-A", "frontend/src"); f.git("commit", "-m", `split ${name}`);
    return f.git("rev-parse", "HEAD");
  };
  const merged = (f: ReturnType<typeof fixture>) => {
    const { merged: catalogs, issues } = analyzePortions(loadPortions(f.locales));
    expect(issues).toEqual([]);
    for (const language of LANGUAGES) expect(fs.existsSync(path.join(f.locales, `${language}.json`))).toBe(false);
    return catalogs;
  };

  describe("history shapes adoption refuses as a whole", () => {
    it.each([
      ["a rebase that drops a merge commit", [], "This rebase drops merge commit"],
      ["a rebase that replays merge commits", ["--rebase-merges"], "The rebase replays merge commits (--rebase-merges)"],
    ])("refuses %s", (_name, flags, message) => {
      const f = fixture();
      legacyMergeCommit(f);
      expect(spawnSync("git", ["rebase", ...flags, "split"], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
      const before = f.git("status", "--porcelain");
      const result = f.adopt();
      expectRefused(f, result, before, message);
      expect(result.stderr).toContain("git rebase --abort");
    });

    it("refuses cherry-picking a merge commit (cherry-pick -m)", () => {
      const f = fixture();
      const merge = legacyMergeCommit(f);
      f.git("checkout", "-q", "split");
      expect(spawnSync("git", ["cherry-pick", "-m", "1", merge], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
      const before = f.git("status", "--porcelain");
      expectRefused(f, f.adopt(), before, "is a merge commit (cherry-pick -m)");
    });

    it("refuses an octopus merge", () => {
      const f = fixture();
      for (const name of ["one", "two"]) {
        f.git("checkout", "-q", "-b", name, "legacy");
        fs.writeFileSync(path.join(f.root, `src/pages/${name}.ts`), "export const x = 1;\n");
        f.git("add", "-A", "frontend/src"); f.git("commit", "-m", name);
      }
      f.git("checkout", "-q", "legacy");
      f.git("merge", "--no-commit", "one", "two");
      const before = f.git("status", "--porcelain");
      expectRefused(f, f.adopt(), before, "MERGE_HEAD names 2 heads (an octopus merge)");
    });

    it("refuses a branch that merged main's split areas into areas it had made itself", () => {
      const f = fixture();
      f.git("checkout", "-q", "legacy");
      for (const language of LANGUAGES) f.catalog(path.join(f.locales, `app/unattributed/${language}.json`), { "Own area": `own ${language}` });
      f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "the branch makes its own split areas");
      commitMergeKeepingMonoliths(f);
      const before = f.git("status", "--porcelain");
      const result = f.adopt("--branch", "HEAD", "--base", f.base, "--onto", f.onto);
      expectRefused(f, result, before, "merged split areas into split areas it had made itself");
      expect(result.stderr).toContain("This history cannot be adopted automatically.");
    });

    it("refuses an --onto that still has the monoliths, or has no split areas", () => {
      const f = fixture();
      const mixed = commitMergeKeepingMonoliths(f);
      const before = f.git("status", "--porcelain");
      expectRefused(f, f.adopt("--branch", f.branch, "--base", f.base, "--onto", mixed), before, `${mixed} (--onto) still has legacy monoliths beside its split areas`);
      expectRefused(f, f.adopt("--branch", f.branch, "--base", f.base, "--onto", f.base), before, `${f.base} (--onto) still has legacy monoliths;`);
      f.git("checkout", "-q", "-b", "no-catalogs", "split");
      f.git("rm", "-q", "-r", "frontend/src/i18n/locales/app");
      f.git("commit", "-q", "-m", "no split areas");
      const bare = f.git("rev-parse", "HEAD");
      f.git("checkout", "-q", "legacy");
      expectRefused(f, f.adopt("--branch", f.branch, "--base", f.base, "--onto", bare), before, `${bare} (--onto) has no split catalogs to adopt onto`);
    });

    it("refuses a legacy merge commit adopted against one of its parents", () => {
      const f = fixture();
      const merge = legacyMergeCommit(f);
      const before = f.git("status", "--porcelain");
      expectRefused(f, f.adopt("--branch", merge, "--base", `${merge}^1`, "--onto", f.onto), before, "is a merge commit and");
    });

    it("refuses a --base that is not an ancestor of --branch (N13)", () => {
      const f = fixture();
      const before = f.git("status", "--porcelain");
      expectRefused(f, f.adopt("--branch", f.branch, "--base", f.onto, "--onto", f.onto), before, `${f.onto} (--base) is not an ancestor of ${f.branch} (--branch)`);
    });

    it("refuses a mixed branch whose --base and merge-base diverge", () => {
      const f = fixture();
      f.git("checkout", "-q", "-b", "side", f.base);
      const side = editMonoliths(f, "side", (catalog, language) => { catalog["Side key"] = `side ${language}`; });
      f.git("checkout", "-q", "legacy");
      f.git("merge", "--no-ff", "-X", "theirs", "-m", "merge side", "side");
      commitMergeKeepingMonoliths(f);
      const before = f.git("status", "--porcelain");
      expectRefused(f, f.adopt("--branch", "HEAD", "--base", side, "--onto", f.onto), before, "do not contain one another, so the reference for the branch's split areas is ambiguous");
    });

    it("refuses a family whose edits have no reference", () => {
      const f = fixture();
      f.git("checkout", "-q", "--orphan", "nofamily", f.base);
      f.git("rm", "-q", "-f", ...FAMILIES.flatMap((family) => LANGUAGES.map((language) => `frontend/src/i18n/locales/${family}-${language}.json`)));
      f.git("commit", "-q", "-m", "monoliths without families");
      const start = f.git("rev-parse", "HEAD");
      f.git("checkout", "-q", f.base, "--", "frontend/src/i18n/locales");
      f.git("add", "-A", "frontend/src"); f.git("commit", "-q", "-m", "families appear");
      const before = f.git("status", "--porcelain");
      expectRefused(f, f.adopt("--branch", "HEAD", "--base", start, "--onto", f.onto), before, `Neither ${start} (--base) nor the merge-base of HEAD and ${f.onto} has the ${FAMILIES[0]} catalogs`);
    });

    it.each(["add", "remove"])("refuses a key the branch changed in one language only (%s): every area keeps key parity", (kind) => {
      const f = fixture();
      f.git("checkout", "-q", "legacy");
      editMonoliths(f, `one-language ${kind}`, (catalog) => { if (kind === "add") catalog["Only en"] = "only en"; else delete catalog.Kept; }, "en");
      const before = f.git("status", "--porcelain");
      const result = f.adopt("--branch", "HEAD", "--base", f.base, "--onto", f.onto);
      expectRefused(f, result, before, `: key parity differs across en/zh-CN/zh-TW`);
      expect(result.stderr).toContain("add or remove the key in every language on the branch");
    });
  });

  describe("shapes adoption models", () => {
    it("prints no notice for merges main already contains below the fork (plain rebase)", () => {
      const f = fixture();
      // Main itself merges a side branch with --no-ff, the legacy branch forks after that merge, then main splits.
      f.git("checkout", "-q", "-b", "side-main", f.base);
      fs.writeFileSync(path.join(f.root, "src/pages/side.ts"), "export const side = 1;\n");
      f.git("add", "-A", "frontend/src"); f.git("commit", "-q", "-m", "side");
      f.git("checkout", "-q", "-b", "main-with-merge", f.base);
      f.git("merge", "-q", "--no-ff", "-m", "main merges side", "side-main");
      const mainMerge = f.git("rev-parse", "HEAD");
      f.git("checkout", "-q", "-b", "late-legacy", mainMerge);
      editMonoliths(f, "b1", (catalog, language) => { catalog["Change me"] = `b1 ${language}`; });
      splitAt(f, "main-with-merge-split", mainMerge);
      f.git("checkout", "-q", "late-legacy");
      expect(spawnSync("git", ["rebase", "main-with-merge-split"], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
      expect(f.git("rev-parse", "REBASE_HEAD^")).toBe(mainMerge);
      const result = f.adopt();
      expect(result.status, result.stderr).toBe(0);
      expect(result.stderr).not.toContain("i18n: notice");
      for (const language of LANGUAGES) expect(merged(f)[language]["Change me"]).toBe(`b1 ${language}`);
    });

    it.each(["forward", "reverse"])("adopts a %s merge (legacy branch checked out, or split branch checked out)", (direction) => {
      const f = fixture();
      if (direction === "reverse") f.git("checkout", "-q", "split");
      expect(spawnSync("git", ["merge", "--no-commit", direction === "reverse" ? "legacy" : "split"], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
      const result = f.adopt();
      expect(result.status, result.stderr).toBe(0);
      if (direction === "reverse") {
        expect(result.stdout).toContain("i18n: MERGE_HEAD has the legacy monoliths and HEAD does not, so MERGE_HEAD is adopted onto HEAD");
        expect(result.stdout).toContain("i18n: adopted MERGE_HEAD");
      } else expect(result.stdout).not.toContain("MERGE_HEAD is adopted onto HEAD");
      f.verify();
    });

    it("guides a reverse merge of a branch that kept the monoliths, and the guidance completes it", () => {
      const f = fixture();
      mainMovesAfterMixedMerge(f);
      f.git("checkout", "-q", "split");
      spawnSync("git", ["merge", "--no-commit", "legacy"], { cwd: f.repo, encoding: "utf8" });
      const before = f.git("status", "--porcelain");
      const refused = f.adopt();
      expect(refused.status).not.toBe(0);
      expect(refused.stderr).toContain("MERGE_HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main");
      expect(refused.stderr).toContain(`  npm run adopt:i18n -- --branch MERGE_HEAD --base ${f.base} --onto HEAD`);
      expect(f.git("status", "--porcelain")).toBe(before);
      const guided = runGuidance(f, refused.stderr);
      expect(guided.status, guided.stderr).toBe(0);
      expectNothingReverted(f);
    });

    it("notes merges a rebase drops below its first replayed commit, and still adopts that commit", () => {
      const f = fixture();
      // a1 lands upstream as the same patch, so git skips it and replays only b1, whose parent is the merge M.
      f.git("checkout", "-q", "-b", "a1", f.base);
      fs.writeFileSync(path.join(f.root, "src/pages/a1.ts"), "export const a1 = 1;\n");
      f.git("add", "-A", "frontend/src"); f.git("commit", "-q", "-m", "a1");
      f.git("checkout", "-q", "-b", "upstream", "split");
      f.git("cherry-pick", "a1");
      f.git("checkout", "-q", "-b", "merged", f.base);
      f.git("merge", "-q", "--no-ff", "-m", "M: merge a1", "a1");
      const merge = f.git("rev-parse", "HEAD");
      editMonoliths(f, "b1", (catalog, language) => { catalog["Change me"] = `b1 ${language}`; });
      expect(spawnSync("git", ["rebase", "upstream"], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
      expect(f.git("rev-parse", "REBASE_HEAD^")).toBe(merge);
      const result = f.adopt();
      expect(result.status, result.stderr).toBe(0);
      expect(result.stderr).toContain(`i18n: notice: this rebase does not replay merge commit ${merge.slice(0, 9)}, so its catalog resolutions are not carried over`);
      expect(result.stderr).toContain("merge main instead (git merge origin/main)");
      const catalogs = merged(f);
      for (const language of LANGUAGES) expect(catalogs[language]).toEqual({ "Change me": `b1 ${language}`, "Remove me": `removed ${language}`, Kept: `kept ${language}` });
    });

    it("places a new key by the branch's source file, after the key's area on onto (N11)", () => {
      const f = fixture();
      commitMergeKeepingMonoliths(f);
      fs.writeFileSync(path.join(f.root, "src/pages/settings.ts"), 'export const copy = ["Settings key", "Change me"];\n');
      editMonoliths(f, "settings copy", (catalog, language) => { catalog["Settings key"] = `settings ${language}`; catalog["Change me"] = `settings change ${language}`; });
      const result = f.adopt("--branch", "HEAD", "--base", f.base, "--onto", f.onto);
      expect(result.status, result.stderr).toBe(0);
      for (const language of LANGUAGES) {
        const portions = loadPortions(f.locales);
        expect(portions.settings?.[language]).toEqual({ "Settings key": `settings ${language}` });
        expect(portions.unattributed?.[language]?.["Change me"]).toBe(`settings change ${language}`);
      }
      merged(f);
    });

    it("cherry-picks a commit that deleted every split area file: its monolith edits land and main keeps every key", () => {
      const f = fixture();
      commitMergeKeepingMonoliths(f);
      f.git("rm", "-q", ...fs.readdirSync(path.join(f.locales, "app"), { withFileTypes: true }).filter((entry) => entry.isDirectory())
        .flatMap((entry) => LANGUAGES.map((language) => `frontend/src/i18n/locales/app/${entry.name}/${language}.json`)));
      const deleted = editMonoliths(f, "delete the split areas and change Kept", (catalog, language) => { catalog.Kept = `kept deleted ${language}`; });
      f.git("checkout", "-q", "split");
      expect(spawnSync("git", ["cherry-pick", deleted], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
      const result = f.adopt();
      expect(result.status, result.stderr).toBe(0);
      const catalogs = merged(f);
      for (const language of LANGUAGES) expect(catalogs[language]).toEqual({ "Change me": `old ${language}`, "Remove me": `removed ${language}`, Kept: `kept deleted ${language}` });
    });

    it("re-lands a reverted commit with cherry-pick, and with explicit refs (N12)", () => {
      const f = fixture();
      f.git("checkout", "-q", "-b", "main3", f.base);
      f.git("merge", "--no-ff", "-m", "land legacy", f.branch);
      f.git("revert", "--no-edit", "-m", "1", "HEAD");
      const main3 = splitAt(f, "main3-split", "main3");
      const expectLanded = () => {
        const catalogs = merged(f);
        for (const language of LANGUAGES) expect(catalogs[language]).toEqual({ "Change me": `new ${language}`, Kept: `kept ${language}`, "Added key": `added ${language}` });
      };
      expect(spawnSync("git", ["cherry-pick", f.branch], { cwd: f.repo, encoding: "utf8" }).status).not.toBe(0);
      const picked = f.adopt();
      expect(picked.status, picked.stderr).toBe(0);
      expectLanded();
      f.git("reset", "-q", "--hard"); f.git("clean", "-fdq", "frontend/src"); f.git("cherry-pick", "--quit");
      expect(f.git("rev-parse", "HEAD")).toBe(main3);
      const explicit = f.adopt("--branch", f.branch, "--base", `${f.branch}^`, "--onto", "HEAD");
      expect(explicit.status, explicit.stderr).toBe(0);
      expectLanded();
    });

    it("applies a later area edit when the branch's monolith change matches the same change on main (N14)", () => {
      const f = fixture();
      f.git("checkout", "-q", "split");
      for (const language of LANGUAGES) {
        const file = path.join(f.locales, "app/billing", `${language}.json`);
        f.catalog(file, { ...JSON.parse(fs.readFileSync(file, "utf8")), Kept: `same ${language}` });
      }
      f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "main changes Kept");
      const onto = f.git("rev-parse", "HEAD");
      f.git("checkout", "-q", "legacy");
      editMonoliths(f, "the branch makes the same change", (catalog, language) => { catalog.Kept = `same ${language}`; });
      spawnSync("git", ["merge", "--no-commit", "-X", "no-renames", onto], { cwd: f.repo, encoding: "utf8" });
      const conflicted = f.git("diff", "--name-only", "--diff-filter=U").split("\n").filter(Boolean);
      if (conflicted.length) f.git("checkout", "--ours", "--", ...conflicted);
      f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "merge main keeping the monoliths");
      for (const language of LANGUAGES) {
        const file = path.join(f.locales, "app/billing", `${language}.json`);
        f.catalog(file, { ...JSON.parse(fs.readFileSync(file, "utf8")), Kept: `area ${language}` });
      }
      f.git("add", "-A", "frontend/src"); f.git("commit", "-m", "the branch edits the area");
      const result = f.adopt("--branch", "HEAD", "--base", f.base, "--onto", onto);
      expect(result.status, result.stderr).toBe(0);
      const catalogs = merged(f);
      for (const language of LANGUAGES) expect(catalogs[language]).toEqual({ "Change me": `new ${language}`, Kept: `area ${language}`, "Added key": `added ${language}` });
    });
  });
});
