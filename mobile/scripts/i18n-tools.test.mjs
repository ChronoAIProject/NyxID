import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterEach, describe, test } from "node:test";
import { canonicalText } from "../../shared/i18n/catalog-core.mjs";
import ts from "typescript";
import { LANGUAGES, auditCatalogs, barrelIdentifiers, barrelText, catalogWarnings, legacyImportIssues } from "./lib/i18n-catalogs.mjs";
import { portionForPath } from "./lib/i18n-portions.mjs";

const mobile = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const temporary = [];
afterEach(() => {
  for (const dir of temporary.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
});
const write = (file, text) => { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text); };
const zh = (phrase, language) => `${phrase} ${language === "zh-CN" ? "简" : "繁"}`;
const catalog = (phrases, language) => Object.fromEntries(phrases.map((phrase) => [phrase, zh(phrase, language)]));

/** A valid split catalog directory with the given areas. */
function catalogDir(areas) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-mobile-i18n-"));
  temporary.push(dir);
  for (const [area, phrases] of Object.entries(areas)) {
    for (const language of LANGUAGES) write(path.join(dir, area, `${language}.json`), canonicalText(catalog(phrases, language)));
  }
  for (const language of LANGUAGES) write(path.join(dir, `${language}.ts`), barrelText(Object.keys(areas), language));
  return dir;
}

describe("mobile area map", () => {
  test("derives feature areas from the directory and keeps navigation and shared", () => {
    assert.equal(portionForPath("src/features/account/Screen.tsx"), "account");
    assert.equal(portionForPath("src/features/brand-new/deep/Thing.tsx"), "brand-new");
    assert.equal(portionForPath("src/app/AppNavigator.tsx"), "navigation");
    assert.equal(portionForPath("src/components/Button.tsx"), "shared");
    assert.equal(portionForPath("src/lib/format.ts"), "shared");
    assert.equal(portionForPath("src/features.ts"), "shared");
  });

  test("fails clearly when a feature directory collides with a reserved area", () => {
    for (const name of ["shared", "unattributed", "navigation"]) {
      assert.throws(() => portionForPath(`src/features/${name}/Screen.tsx`), new RegExp(`src/features/${name} collides with the reserved catalog area "${name}"`));
    }
  });
});

describe("mobile barrel identifiers", () => {
  const areas = ["account", "foo-bar", "fooBar", "foo_bar", "delete", "default", "new", "2fa", "3-d", "---", "catalog", "Account"];

  test("are valid, collision-free names for hyphenated, reserved-word, digit-leading and colliding areas", () => {
    const names = barrelIdentifiers(areas);
    assert.deepEqual(Object.fromEntries(names), {
      "---": "areaCatalog",
      "2fa": "area2faCatalog",
      "3-d": "area3DCatalog",
      Account: "accountCatalog",
      account: "accountCatalog2",
      catalog: "catalogCatalog",
      default: "defaultCatalog",
      delete: "deleteCatalog",
      "foo-bar": "fooBarCatalog",
      fooBar: "fooBarCatalog2",
      foo_bar: "fooBarCatalog3",
      new: "newCatalog",
    });
    assert.equal(new Set(names.values()).size, areas.length);
    for (const name of names.values()) assert.match(name, /^[A-Za-z][A-Za-z0-9]*$/);
  });

  test("produce a barrel that parses and imports every area exactly once", () => {
    const text = barrelText(areas, "zh-TW");
    const { diagnostics } = ts.transpileModule(text, { reportDiagnostics: true, compilerOptions: { module: ts.ModuleKind.ESNext } });
    assert.deepEqual(diagnostics, []);
    const imports = [...text.matchAll(/^import (\w+) from "\.\/(.+)\/zh-TW\.json";$/gm)];
    assert.deepEqual(imports.map((match) => match[2]).sort(), [...areas].sort());
    assert.equal(new Set(imports.map((match) => match[1])).size, areas.length);
    for (const [, name] of imports) assert.match(text, new RegExp(`^  \\.\\.\\.${name},$`, "m"));
  });
});

describe("mobile catalog audit", () => {
  test("accepts a valid split", () => {
    assert.deepEqual(auditCatalogs(catalogDir({ account: ["Save {{name}}"], auth: ["Sign in"] })), []);
  });

  test("names pnpm adopt:i18n for a legacy monolith", () => {
    const dir = catalogDir({ account: ["Save"] });
    write(path.join(dir, "zh-TW.json"), "{}\n");
    assert.deepEqual(auditCatalogs(dir), ["legacy monolith zh-TW.json is present; run pnpm adopt:i18n (see docs/MULTILINGUAL.md, Native app)"]);
  });

  test("fails when a barrel misses an area import", () => {
    const dir = catalogDir({ account: ["Save"], auth: ["Sign in"] });
    write(path.join(dir, "zh-CN.ts"), barrelText(["account"], "zh-CN"));
    assert.deepEqual(auditCatalogs(dir), ["zh-CN.ts: barrel must import exactly the area catalogs (account, auth); run pnpm sync:i18n"]);
  });

  test("reports unsorted files, parity, placeholders, empty values and differing duplicates", () => {
    const dir = catalogDir({ account: ["Save"], auth: ["Sign in"] });
    write(path.join(dir, "account/zh-CN.json"), '{\n  "b": "1",\n  "a": "2"\n}\n');
    write(path.join(dir, "auth/zh-CN.json"), canonicalText({ "Sign in": "", "Hi {{name}}": "你好", a: "different" }));
    write(path.join(dir, "auth/zh-TW.json"), canonicalText({ "Sign in": "登入", "Hi {{name}}": "你好 {{name}}", a: "x" }));
    write(path.join(dir, "account/zh-TW.json"), canonicalText({ a: "2", b: "1" }));
    write(path.join(dir, "notes.txt"), "");
    assert.deepEqual(auditCatalogs(dir).sort(), [
      "account/zh-CN.json: non-canonical catalog; run pnpm sync:i18n",
      "auth/zh-CN.json: empty translation: Sign in",
      "auth/zh-CN.json: placeholders differ from the phrase: Hi {{name}}",
      "mobile duplicate key has differing values: a (account, auth)",
      "unexpected file notes.txt; catalogs live in <area>/{zh-CN,zh-TW}.json",
    ]);
  });

  test("warns about identical cross-area duplicates without failing", () => {
    const dir = catalogDir({ account: ["Save", "Profile"], auth: ["Save", "Sign in"] });
    assert.deepEqual(auditCatalogs(dir), []);
    assert.deepEqual(catalogWarnings(dir), ["i18n: identical duplicate Save in account, auth; run pnpm sync:i18n"]);
  });

  test("ignores stray non-JSON files in and beside area directories", () => {
    const dir = catalogDir({ account: ["Save"] });
    write(path.join(dir, "account/.DS_Store"), "");
    write(path.join(dir, "emptied/.DS_Store"), "");
    write(path.join(dir, ".DS_Store"), "");
    assert.deepEqual(auditCatalogs(dir), []);
    write(path.join(dir, "account/en.json"), "{}\n");
    assert.deepEqual(auditCatalogs(dir), ["account/en.json: unexpected file; areas hold only zh-CN.json and zh-TW.json"]);
  });

  test("flags an empty area file", () => {
    const dir = catalogDir({ account: ["Save"], auth: [] });
    assert.deepEqual(auditCatalogs(dir), ["auth/zh-CN.json: empty area; run pnpm sync:i18n", "auth/zh-TW.json: empty area; run pnpm sync:i18n"]);
  });

  test("reports per-area language parity", () => {
    const dir = catalogDir({ account: ["Save"] });
    write(path.join(dir, "account/zh-TW.json"), canonicalText({ Other: "其他" }));
    assert.deepEqual(auditCatalogs(dir), ["mobile/account: key parity differs across zh-CN/zh-TW"]);
  });
});

/** A throwaway repository with the mobile scripts, the shared core and a legacy monolith. */
function repository() {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-mobile-adopt-"));
  temporary.push(repo);
  const root = path.join(repo, "mobile");
  const locales = path.join(root, "src/i18n/locales");
  const git = (...args) => execFileSync("git", args, { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  for (const file of ["adopt-i18n.mjs", "sync-i18n.mjs", "lib/i18n-catalogs.mjs", "lib/i18n-portions.mjs"]) write(path.join(root, "scripts", file), fs.readFileSync(path.join(mobile, "scripts", file), "utf8"));
  write(path.join(repo, "shared/i18n/catalog-core.mjs"), fs.readFileSync(path.join(mobile, "../shared/i18n/catalog-core.mjs"), "utf8"));
  fs.symlinkSync(path.join(mobile, "node_modules"), path.join(root, "node_modules"));
  git("init", "-q", "-b", "legacy");
  git("config", "user.name", "i18n rehearsal");
  git("config", "user.email", "i18n@example.test");
  git("config", "core.hooksPath", "/dev/null");
  git("config", "commit.gpgsign", "false");
  write(path.join(repo, ".gitignore"), "node_modules\n");
  for (const language of LANGUAGES) write(path.join(locales, `${language}.json`), JSON.stringify(catalog(["Save", "Sign in", "Change me", "Remove me"], language), null, 2));
  write(path.join(root, "src/features/account/Screen.tsx"), 'export const a = [t("Save"), t("Change me")];\n');
  write(path.join(root, "src/features/auth/Login.tsx"), 'export const a = [t("Sign in"), t("Save")];\n');
  write(path.join(root, "src/features/auth/Login.test.tsx"), 'export const a = t("Remove me");\n');
  // Type-checks only the in-flight legacy import added by the branch below.
  write(path.join(root, "tsconfig.json"), `${JSON.stringify({
    compilerOptions: { module: "preserve", moduleResolution: "bundler", resolveJsonModule: true, esModuleInterop: true, strict: true, noUncheckedIndexedAccess: true, noEmit: true, skipLibCheck: true, target: "ESNext" },
    files: ["src/features/auth/legacyCatalog.test.ts"],
  }, null, 2)}\n`);
  git("add", "."); git("commit", "-q", "-m", "legacy layout");
  const base = git("rev-parse", "HEAD");
  const run = (script, ...args) => spawnSync(process.execPath, [path.join(root, "scripts", script), ...args], { cwd: root, encoding: "utf8" });
  return { repo, root, locales, git, run, base };
}

const readCatalog = (locales, area, language) => JSON.parse(fs.readFileSync(path.join(locales, area, `${language}.json`), "utf8"));

describe("mobile adopt and sync tools", () => {
  test("splits the legacy monolith by literal usage, ignoring tests, and is repeatable", () => {
    const r = repository();
    const adopted = r.run("adopt-i18n.mjs", "--branch", r.base, "--base", "empty", "--onto", r.base);
    assert.equal(adopted.status, 0, adopted.stderr);
    assert.deepEqual(fs.readdirSync(r.locales).sort(), ["account", "auth", "shared", "unattributed", "zh-CN.ts", "zh-TW.ts"]);
    assert.deepEqual(readCatalog(r.locales, "account", "zh-TW"), { "Change me": zh("Change me", "zh-TW") });
    assert.deepEqual(readCatalog(r.locales, "auth", "zh-CN"), { "Sign in": zh("Sign in", "zh-CN") });
    assert.deepEqual(readCatalog(r.locales, "shared", "zh-CN"), { Save: zh("Save", "zh-CN") });
    assert.deepEqual(readCatalog(r.locales, "unattributed", "zh-CN"), { "Remove me": zh("Remove me", "zh-CN") });
    assert.deepEqual(auditCatalogs(r.locales), []);
    r.git("add", "-A"); r.git("commit", "-q", "-m", "split");
    assert.equal(r.run("adopt-i18n.mjs", "--branch", r.base, "--base", "empty", "--onto", "HEAD").status, 0);
    assert.equal(r.run("sync-i18n.mjs").status, 0);
    assert.equal(r.git("status", "--porcelain"), "");
  });

  for (const mode of ["merge", "rebase"]) {
    test(`adopts an old-layout branch during a real ${mode}`, () => {
      const r = repository();
      r.git("checkout", "-q", "-b", "split");
      assert.equal(r.run("adopt-i18n.mjs", "--branch", r.base, "--base", "empty", "--onto", r.base).status, 0);
      r.git("add", "-A"); r.git("commit", "-q", "-m", "split");
      r.git("checkout", "-q", "legacy");
      for (const language of LANGUAGES) {
        const next = catalog(["Save", "Sign in", "Change me", "Added"], language);
        next["Change me"] = `changed ${language}`;
        write(path.join(r.locales, `${language}.json`), JSON.stringify(next, null, 2));
      }
      write(path.join(r.root, "src/features/auth/Added.tsx"), 'export const a = t("Added");\n');
      write(path.join(r.root, "src/features/auth/legacyCatalog.test.ts"), 'import zh from "../../i18n/locales/zh-CN.json";\n\nexport const save: string = zh["Save"];\n');
      r.git("add", "-A"); r.git("commit", "-q", "-m", "legacy delta");
      const started = spawnSync("git", mode === "merge" ? ["merge", "--no-commit", "split"] : ["rebase", "split"], { cwd: r.repo, encoding: "utf8" });
      assert.notEqual(started.status, 0);
      const adopted = r.run("adopt-i18n.mjs");
      assert.equal(adopted.status, 0, adopted.stderr);
      assert.match(adopted.stdout, /i18n: rewrote src\/features\/auth\/legacyCatalog\.test\.ts:1 \.\.\/\.\.\/i18n\/locales\/zh-CN\.json -> \.\.\/\.\.\/i18n\/locales\/zh-CN\n/);
      assert.match(adopted.stdout, /git add src\/features\/auth\/legacyCatalog\.test\.ts\n/);
      assert.match(adopted.stdout, /run pnpm typecheck/);
      assert.equal(fs.readFileSync(path.join(r.root, "src/features/auth/legacyCatalog.test.ts"), "utf8"), 'import zh from "../../i18n/locales/zh-CN";\n\nexport const save: string = zh["Save"];\n');
      const typecheck = spawnSync(process.execPath, [path.join(r.root, "node_modules/typescript/bin/tsc"), "-p", "tsconfig.json"], { cwd: r.root, encoding: "utf8" });
      assert.equal(typecheck.status, 0, typecheck.stdout);
      assert.deepEqual(legacyImportIssues(r.root), []);
      r.git("add", "-A", "mobile/src/i18n/locales", "mobile/src/features/auth/legacyCatalog.test.ts");
      assert.equal(r.git("diff", "--name-only", "--diff-filter=U"), "");
      if (mode === "rebase") r.git("-c", "core.editor=true", "rebase", "--continue");
      else r.git("commit", "-q", "-m", "resolved");
      assert.deepEqual(auditCatalogs(r.locales), []);
      assert.deepEqual(readCatalog(r.locales, "account", "zh-CN"), { "Change me": "changed zh-CN" });
      assert.deepEqual(readCatalog(r.locales, "auth", "zh-TW"), { Added: zh("Added", "zh-TW"), "Sign in": zh("Sign in", "zh-TW") });
      assert.equal(fs.existsSync(path.join(r.locales, "unattributed")), false);
    });
  }

  /** Commit the split on `split`, then a legacy catalog delta on `legacy`. */
  const splitThenLegacyDelta = (r) => {
    r.git("checkout", "-q", "-b", "split");
    assert.equal(r.run("adopt-i18n.mjs", "--branch", r.base, "--base", "empty", "--onto", r.base).status, 0);
    r.git("add", "-A"); r.git("commit", "-q", "-m", "split");
    const onto = r.git("rev-parse", "HEAD");
    r.git("checkout", "-q", "legacy");
    for (const language of LANGUAGES) {
      const next = catalog(["Save", "Sign in", "Change me", "Remove me", "Added"], language);
      next["Change me"] = `changed ${language}`;
      write(path.join(r.locales, `${language}.json`), JSON.stringify(next, null, 2));
    }
    write(path.join(r.root, "src/features/auth/Added.tsx"), 'export const a = t("Added");\n');
    r.git("add", "-A"); r.git("commit", "-q", "-m", "legacy delta");
    return { onto, branch: r.git("rev-parse", "HEAD") };
  };
  const expectDeltaAdopted = (r) => {
    assert.deepEqual(auditCatalogs(r.locales), []);
    assert.deepEqual(readCatalog(r.locales, "account", "zh-CN"), { "Change me": "changed zh-CN" });
    assert.deepEqual(readCatalog(r.locales, "auth", "zh-TW"), { Added: zh("Added", "zh-TW"), "Sign in": zh("Sign in", "zh-TW") });
    for (const language of LANGUAGES) assert.equal(fs.existsSync(path.join(r.locales, `${language}.json`)), false);
  };

  /** Merge split into legacy, keep the legacy monoliths, and commit (M). */
  const commitMergeKeepingMonoliths = (r) => {
    // The fixture is small enough to trip rename detection; the real split is not.
    spawnSync("git", ["merge", "--no-commit", "-X", "no-renames", "split"], { cwd: r.repo, encoding: "utf8" });
    const conflicted = r.git("diff", "--name-only", "--diff-filter=U").split("\n").filter(Boolean);
    r.git("checkout", "--ours", "--", ...conflicted);
    r.git("add", "-A"); r.git("commit", "-q", "-m", "merge that kept the monoliths");
    for (const language of LANGUAGES) assert.equal(fs.existsSync(path.join(r.locales, `${language}.json`)), true);
    return r.git("rev-parse", "HEAD");
  };

  test("uses a mixed branch's monoliths so its legacy edits land and nothing is lost", () => {
    const r = repository();
    const { onto } = splitThenLegacyDelta(r);
    commitMergeKeepingMonoliths(r);
    const adopted = r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", r.base, "--onto", onto);
    assert.equal(adopted.status, 0, adopted.stderr);
    expectDeltaAdopted(r);
  });

  /** After M, main moves on: it changes "Save" and adds "Main new" in shared. */
  const mainMovesAfterMixedMerge = (r, { beforeMerge = () => {}, afterMerge = () => {} } = {}) => {
    const { onto } = splitThenLegacyDelta(r);
    beforeMerge();
    const mixed = commitMergeKeepingMonoliths(r);
    afterMerge();
    r.git("checkout", "-q", "split");
    for (const language of LANGUAGES) {
      const file = path.join(r.locales, "shared", `${language}.json`);
      write(file, canonicalText({ ...JSON.parse(fs.readFileSync(file, "utf8")), Save: `save v2 ${language}`, "Main new": `main ${language}` }));
    }
    r.git("add", "-A"); r.git("commit", "-q", "-m", "main moves on");
    const moved = r.git("rev-parse", "HEAD");
    r.git("checkout", "-q", "legacy");
    return { onto, mixed, moved };
  };
  /** Run the guidance command adopt printed, through this fixture's script. */
  const runGuidance = (r, stderr) => {
    const command = stderr.split("\n").find((line) => line.startsWith("  pnpm adopt:i18n --branch"));
    assert.ok(command, stderr);
    return spawnSync("sh", ["-c", command.trim().replace("pnpm adopt:i18n", `"${process.execPath}" scripts/adopt-i18n.mjs`)], { cwd: r.root, encoding: "utf8" });
  };
  /** The branch's legacy delta on top of main's later changes, with nothing of main reverted. */
  const expectNothingReverted = (r, extra = () => ({})) => {
    assert.deepEqual(auditCatalogs(r.locales), []);
    const merged = (language) => Object.assign({}, ...fs.readdirSync(r.locales, { withFileTypes: true })
      .filter((entry) => entry.isDirectory()).map((entry) => readCatalog(r.locales, entry.name, language)));
    for (const language of LANGUAGES) {
      assert.deepEqual(merged(language), {
        Save: `save v2 ${language}`, "Main new": `main ${language}`, "Sign in": zh("Sign in", language),
        "Change me": `changed ${language}`, "Remove me": zh("Remove me", language), Added: zh("Added", language), ...extra(language),
      });
      assert.equal(fs.existsSync(path.join(r.locales, `${language}.json`)), false);
    }
  };

  test("refuses a mixed branch against a split-only --base, and its guidance adopts without reverting main", () => {
    const r = repository();
    const { onto, mixed, moved } = mainMovesAfterMixedMerge(r);
    const before = r.git("status", "--porcelain");
    const refused = r.run("adopt-i18n.mjs", "--branch", mixed, "--base", onto, "--onto", moved);
    assert.notEqual(refused.status, 0);
    assert.ok(refused.stderr.includes(`No files written. ${mixed} (--branch) keeps the legacy monoliths beside split areas it inherited from main, so its own edits live in the monoliths (and any later edits to those areas). ${onto} (--base) has only the split areas`), refused.stderr);
    assert.ok(refused.stderr.includes(`  pnpm adopt:i18n --branch ${mixed} --base ${r.base} --onto ${moved}`), refused.stderr);
    assert.ok(!refused.stderr.includes("^1"), refused.stderr);
    assert.equal(r.git("status", "--porcelain"), before);
    const guided = runGuidance(r, refused.stderr);
    assert.equal(guided.status, 0, guided.stderr);
    expectNothingReverted(r);
  });

  test("refuses the auto-detected merge of a branch that kept the monoliths, and its guidance completes the merge", () => {
    const r = repository();
    mainMovesAfterMixedMerge(r);
    spawnSync("git", ["merge", "--no-commit", "split"], { cwd: r.repo, encoding: "utf8" });
    assert.equal(fs.existsSync(path.join(r.repo, ".git/MERGE_HEAD")), true);
    const before = r.git("status", "--porcelain");
    const refused = r.run("adopt-i18n.mjs");
    assert.notEqual(refused.status, 0);
    assert.ok(refused.stderr.includes("HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main"), refused.stderr);
    assert.ok(refused.stderr.includes(`  pnpm adopt:i18n --branch HEAD --base ${r.base} --onto MERGE_HEAD`), refused.stderr);
    assert.equal(r.git("status", "--porcelain"), before);
    const guided = runGuidance(r, refused.stderr);
    assert.equal(guided.status, 0, guided.stderr);
    expectNothingReverted(r);
    r.git("add", "-A", "mobile/src/i18n/locales"); r.git("commit", "-q", "-m", "second merge");
    expectNothingReverted(r);
  });

  test("adopts a branch that edited its monoliths after the merge that kept them, following the printed guidance", () => {
    const r = repository();
    mainMovesAfterMixedMerge(r, {
      // Main changes "Save" before the branch merges it (and again afterwards).
      beforeMerge: () => {
        r.git("checkout", "-q", "split");
        for (const language of LANGUAGES) {
          const file = path.join(r.locales, "shared", `${language}.json`);
          write(file, canonicalText({ ...JSON.parse(fs.readFileSync(file, "utf8")), Save: `save v1 ${language}` }));
        }
        r.git("add", "-A"); r.git("commit", "-q", "-m", "main changes Save");
        r.git("checkout", "-q", "legacy");
      },
      // X: after the merge that kept the monoliths, the branch edits them again.
      afterMerge: () => {
        for (const language of LANGUAGES) {
          const file = path.join(r.locales, `${language}.json`);
          write(file, JSON.stringify({ ...JSON.parse(fs.readFileSync(file, "utf8")), "X key": `x ${language}` }, null, 2));
        }
        r.git("add", "-A"); r.git("commit", "-q", "-m", "X edits the monoliths");
      },
    });
    assert.equal(r.git("rev-list", "--parents", "-n", "1", "HEAD").split(" ").length, 2);
    // The branch merges main again; adopt, then follow exactly what it prints.
    spawnSync("git", ["merge", "--no-commit", "split"], { cwd: r.repo, encoding: "utf8" });
    const before = r.git("status", "--porcelain");
    const refused = r.run("adopt-i18n.mjs");
    assert.notEqual(refused.status, 0);
    assert.ok(refused.stderr.includes("HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main"), refused.stderr);
    assert.ok(!refused.stderr.includes("^1"), refused.stderr);
    assert.ok(refused.stderr.includes(`  pnpm adopt:i18n --branch HEAD --base ${r.base} --onto MERGE_HEAD`), refused.stderr);
    assert.equal(r.git("status", "--porcelain"), before);
    const guided = runGuidance(r, refused.stderr);
    assert.equal(guided.status, 0, guided.stderr);
    const xKey = (language) => ({ "X key": `x ${language}` });
    expectNothingReverted(r, xKey);
    r.git("add", "-A", "mobile/src/i18n/locales"); r.git("commit", "-q", "-m", "merge main again");
    expectNothingReverted(r, xKey);
  });

  test("refuses a split-only branch against a legacy-only base", () => {
    const r = repository();
    const { onto } = splitThenLegacyDelta(r);
    const result = r.run("adopt-i18n.mjs", "--branch", onto, "--base", r.base, "--onto", onto);
    assert.notEqual(result.status, 0);
    assert.ok(result.stderr.includes(`No files written. ${onto} (--branch) has no legacy monoliths, so it is not a branch from before the split and there is nothing to adopt`), result.stderr);
  });


  test("refuses partial monoliths without writing", () => {
    const r = repository();
    const { onto } = splitThenLegacyDelta(r);
    r.git("rm", "-q", "mobile/src/i18n/locales/zh-TW.json"); r.git("commit", "-q", "-m", "partial");
    const result = r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", r.base, "--onto", onto);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /No files written\. HEAD \(--branch\) has only some legacy monoliths \(zh-CN\.json\)/);
    assert.equal(r.git("status", "--porcelain"), "");
  });

  test("adopts a conflicted cherry-pick and explains git am", () => {
    const r = repository();
    const { branch } = splitThenLegacyDelta(r);
    r.git("checkout", "-q", "split");
    assert.notEqual(spawnSync("git", ["cherry-pick", branch], { cwd: r.repo, encoding: "utf8" }).status, 0);
    const picked = r.run("adopt-i18n.mjs");
    assert.equal(picked.status, 0, picked.stderr);
    assert.match(picked.stdout, /adopted CHERRY_PICK_HEAD \(base CHERRY_PICK_HEAD\^\) onto HEAD/);
    expectDeltaAdopted(r);

    const am = repository();
    const delta = splitThenLegacyDelta(am);
    const patch = path.join(am.repo, "legacy.patch");
    fs.writeFileSync(patch, am.git("format-patch", "-1", "--stdout", delta.branch));
    am.git("checkout", "-q", "split");
    assert.notEqual(spawnSync("git", ["am", "-3", patch], { cwd: am.repo, encoding: "utf8" }).status, 0);
    const result = am.run("adopt-i18n.mjs");
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /git am is in progress; .*\(pnpm adopt:i18n --branch <legacy-commit> --base <legacy-commit>\^ --onto HEAD\)/);
    assert.doesNotMatch(result.stderr, /bad revision/);
  });

  test("refuses to place keys when a feature directory uses a reserved area name", () => {
    const r = repository();
    write(path.join(r.root, "src/features/shared/Screen.tsx"), 'export const a = t("Remove me");\n');
    r.git("add", "-A"); r.git("commit", "-q", "-m", "reserved feature");
    const result = r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", "empty", "--onto", "HEAD");
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /src\/features\/shared collides with the reserved catalog area "shared"/);
    assert.equal(fs.existsSync(path.join(r.locales, "shared")), false);
  });

  test("reports and rewrites remaining legacy imports outside a merge or rebase with --imports-only", () => {
    const r = repository();
    assert.equal(r.run("adopt-i18n.mjs", "--branch", r.base, "--base", "empty", "--onto", r.base).status, 0);
    write(path.join(r.root, "src/features/auth/late.test.tsx"), 'jest.mock("../../i18n/locales/zh-TW.json");\nconst zh = require("../../i18n/locales/zh-CN.json");\nimport data from "./data.json";\n');
    write(path.join(r.root, "src/i18n/translate.ts"), 'import simplified from "./locales/zh-CN.json";\n');
    assert.deepEqual(legacyImportIssues(r.root), [
      "src/features/auth/late.test.tsx:1: legacy catalog import ../../i18n/locales/zh-TW.json; use ../../i18n/locales/zh-TW (pnpm adopt:i18n rewrites it; outside a merge or rebase run pnpm adopt:i18n --imports-only)",
      "src/features/auth/late.test.tsx:2: legacy catalog import ../../i18n/locales/zh-CN.json; use ../../i18n/locales/zh-CN (pnpm adopt:i18n rewrites it; outside a merge or rebase run pnpm adopt:i18n --imports-only)",
      "src/i18n/translate.ts:1: legacy catalog import ./locales/zh-CN.json; use ./locales/zh-CN (pnpm adopt:i18n rewrites it; outside a merge or rebase run pnpm adopt:i18n --imports-only)",
    ]);
    const result = r.run("adopt-i18n.mjs", "--imports-only");
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /i18n: rewrote src\/i18n\/translate\.ts:1 \.\/locales\/zh-CN\.json -> \.\/locales\/zh-CN\n/);
    assert.equal(fs.readFileSync(path.join(r.root, "src/features/auth/late.test.tsx"), "utf8"), 'jest.mock("../../i18n/locales/zh-TW");\nconst zh = require("../../i18n/locales/zh-CN");\nimport data from "./data.json";\n');
    assert.deepEqual(legacyImportIssues(r.root), []);
    assert.equal(r.run("adopt-i18n.mjs", "--imports-only").stdout, "i18n: run pnpm typecheck to type-check the result.\n");
  });

  test("removes JSON import attributes when a legacy import is rewritten to a barrel", () => {
    const r = repository();
    assert.equal(r.run("adopt-i18n.mjs", "--branch", r.base, "--base", "empty", "--onto", r.base).status, 0);
    write(path.join(r.root, "src/i18n/attributes.test.ts"), [
      'import cn from "./locales/zh-CN.json" with { type: "json" };',
      "import tw from './locales/zh-TW.json' assert { type: 'json' };",
      'const lazy = await import("./locales/zh-CN.json", { with: { type: "json" } });',
      'import data from "./data.json" with { type: "json" };',
      "",
    ].join("\n"));
    const result = r.run("adopt-i18n.mjs", "--imports-only");
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /i18n: rewrote src\/i18n\/attributes\.test\.ts:1 \.\/locales\/zh-CN\.json -> \.\/locales\/zh-CN \(removed its JSON import attributes\)\n/);
    assert.equal(fs.readFileSync(path.join(r.root, "src/i18n/attributes.test.ts"), "utf8"), [
      'import cn from "./locales/zh-CN";',
      "import tw from './locales/zh-TW';",
      'const lazy = await import("./locales/zh-CN");',
      'import data from "./data.json" with { type: "json" };',
      "",
    ].join("\n"));
  });

  test("rejects a divergent value without writing files", () => {
    const r = repository();
    r.git("checkout", "-q", "-b", "split");
    assert.equal(r.run("adopt-i18n.mjs", "--branch", r.base, "--base", "empty", "--onto", r.base).status, 0);
    const file = path.join(r.locales, "account/zh-TW.json");
    write(file, canonicalText({ "Change me": "onto value" }));
    r.git("add", "-A"); r.git("commit", "-q", "-m", "split with onto change");
    r.git("checkout", "-q", "legacy");
    write(path.join(r.locales, "zh-TW.json"), JSON.stringify({ ...catalog(["Save", "Sign in", "Remove me"], "zh-TW"), "Change me": "branch value" }));
    r.git("commit", "-q", "-am", "branch change");
    const branch = r.git("rev-parse", "HEAD");
    r.git("checkout", "-q", "split");
    const result = r.run("adopt-i18n.mjs", "--branch", branch, "--base", r.base, "--onto", "HEAD");
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /No files written[\s\S]*mobile\/zh-TW: changed key diverged: Change me/);
    assert.equal(r.git("status", "--porcelain"), "");
  });

  test("sync consolidates identical duplicates into shared, rejects differing values and legacy files", () => {
    const r = repository();
    assert.equal(r.run("adopt-i18n.mjs", "--branch", r.base, "--base", "empty", "--onto", r.base).status, 0);
    for (const language of LANGUAGES) write(path.join(r.locales, "auth", `${language}.json`), JSON.stringify({ "Change me": zh("Change me", language), "Sign in": zh("Sign in", language) }));
    // A stray file keeps the emptied account directory alive after its JSON is pruned.
    write(path.join(r.locales, "account/.DS_Store"), "");
    write(path.join(r.locales, ".DS_Store"), "");
    const synced = r.run("sync-i18n.mjs");
    assert.equal(synced.status, 0, synced.stderr);
    assert.equal(synced.stderr, "i18n: identical duplicate Change me in account, auth; moved to shared\n");
    assert.deepEqual(Object.keys(readCatalog(r.locales, "shared", "zh-TW")), ["Change me", "Save"]);
    assert.deepEqual(fs.readdirSync(path.join(r.locales, "account")), [".DS_Store"]);
    for (const language of LANGUAGES) {
      const barrel = fs.readFileSync(path.join(r.locales, `${language}.ts`), "utf8");
      assert.doesNotMatch(barrel, /\.\/account\//);
      assert.equal(barrel, barrelText(["auth", "shared", "unattributed"], language));
    }
    assert.deepEqual(auditCatalogs(r.locales), []);
    write(path.join(r.locales, "auth/zh-TW.json"), canonicalText({ Save: "different", "Sign in": zh("Sign in", "zh-TW") }));
    write(path.join(r.locales, "auth/zh-CN.json"), canonicalText({ Save: zh("Save", "zh-CN"), "Sign in": zh("Sign in", "zh-CN") }));
    const before = fs.readFileSync(path.join(r.locales, "auth/zh-TW.json"), "utf8");
    const rejected = r.run("sync-i18n.mjs");
    assert.notEqual(rejected.status, 0);
    assert.match(rejected.stderr, /mobile duplicate key has differing values: Save \(auth, shared\)/);
    assert.equal(fs.readFileSync(path.join(r.locales, "auth/zh-TW.json"), "utf8"), before);
    write(path.join(r.locales, "zh-CN.json"), "{}\n");
    assert.match(r.run("sync-i18n.mjs").stderr, /legacy monolith src\/i18n\/locales\/zh-CN\.json is present; run pnpm adopt:i18n/);
  });

  /** Apply `edit` to every monolith (or one language's) and commit. */
  const editMonoliths = (r, message, edit, only) => {
    for (const language of only ? [only] : LANGUAGES) {
      const file = path.join(r.locales, `${language}.json`);
      const next = JSON.parse(fs.readFileSync(file, "utf8"));
      edit(next, language);
      write(file, JSON.stringify(next, null, 2));
    }
    r.git("add", "-A"); r.git("commit", "-q", "-m", message);
    return r.git("rev-parse", "HEAD");
  };
  /** A legacy merge commit: a side branch from the legacy tip edits the monoliths, legacy merges it with --no-ff. */
  const legacyMergeCommit = (r) => {
    r.git("checkout", "-q", "-b", "side", "legacy");
    editMonoliths(r, "side edits the monoliths", (next, language) => { next["Side key"] = `side ${language}`; });
    r.git("checkout", "-q", "legacy");
    write(path.join(r.root, "src/features/auth/LegacyOnly.tsx"), "export const x = 1;\n");
    r.git("add", "-A"); r.git("commit", "-q", "-m", "legacy moves on");
    r.git("merge", "-q", "--no-ff", "-m", "merge side", "side");
    return r.git("rev-parse", "HEAD");
  };
  /** A refusal that writes nothing and prints no command to run instead. */
  const expectRefused = (r, result, before, message) => {
    assert.notEqual(result.status, 0);
    assert.ok(result.stderr.includes("No files written."), result.stderr);
    assert.ok(result.stderr.includes(message), result.stderr);
    assert.ok(!result.stderr.includes("  pnpm adopt:i18n --branch"), result.stderr);
    assert.equal(r.git("status", "--porcelain"), before);
  };
  const mergedCatalog = (r, language) => Object.assign({}, ...fs.readdirSync(r.locales, { withFileTypes: true })
    .filter((entry) => entry.isDirectory()).map((entry) => readCatalog(r.locales, entry.name, language)));

  for (const [name, flags, message] of [
    ["drops a merge commit", [], "This rebase drops merge commit"],
    ["replays merge commits", ["--rebase-merges"], "The rebase replays merge commits (--rebase-merges)"],
  ]) {
    test(`refuses a rebase that ${name}`, () => {
      const r = repository();
      splitThenLegacyDelta(r);
      legacyMergeCommit(r);
      assert.notEqual(spawnSync("git", ["rebase", ...flags, "split"], { cwd: r.repo, encoding: "utf8" }).status, 0);
      const before = r.git("status", "--porcelain");
      const result = r.run("adopt-i18n.mjs");
      expectRefused(r, result, before, message);
      assert.ok(result.stderr.includes("git rebase --abort") && result.stderr.includes("run pnpm adopt:i18n,"), result.stderr);
    });
  }

  test("refuses cherry-picking a merge commit (cherry-pick -m) and an octopus merge", () => {
    const r = repository();
    splitThenLegacyDelta(r);
    const merge = legacyMergeCommit(r);
    r.git("checkout", "-q", "split");
    assert.notEqual(spawnSync("git", ["cherry-pick", "-m", "1", merge], { cwd: r.repo, encoding: "utf8" }).status, 0);
    let before = r.git("status", "--porcelain");
    expectRefused(r, r.run("adopt-i18n.mjs"), before, "is a merge commit (cherry-pick -m)");
    r.git("cherry-pick", "--abort");
    for (const branch of ["one", "two"]) {
      r.git("checkout", "-q", "-b", branch, "legacy");
      write(path.join(r.root, `src/features/auth/${branch}.ts`), "export const x = 1;\n");
      r.git("add", "-A"); r.git("commit", "-q", "-m", branch);
    }
    r.git("checkout", "-q", "legacy");
    r.git("merge", "-q", "--no-commit", "one", "two");
    before = r.git("status", "--porcelain");
    expectRefused(r, r.run("adopt-i18n.mjs"), before, "MERGE_HEAD names 2 heads (an octopus merge)");
  });

  test("refuses a branch that merged main's split areas into areas it had made itself", () => {
    const r = repository();
    const { onto } = splitThenLegacyDelta(r);
    for (const language of LANGUAGES) write(path.join(r.locales, "own", `${language}.json`), canonicalText({ "Own area": `own ${language}` }));
    r.git("add", "-A"); r.git("commit", "-q", "-m", "the branch makes its own split areas");
    commitMergeKeepingMonoliths(r);
    const before = r.git("status", "--porcelain");
    const result = r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", r.base, "--onto", onto);
    expectRefused(r, result, before, "merged split areas into split areas it had made itself");
    assert.ok(result.stderr.includes("This history cannot be adopted automatically."), result.stderr);
  });

  test("refuses an --onto that is not plain split, a merge commit against its parent, and a --base off the branch", () => {
    const r = repository();
    const { onto, branch } = splitThenLegacyDelta(r);
    const merge = legacyMergeCommit(r);
    const before = r.git("status", "--porcelain");
    expectRefused(r, r.run("adopt-i18n.mjs", "--branch", branch, "--base", r.base, "--onto", r.base), before, `${r.base} (--onto) still has legacy monoliths;`);
    expectRefused(r, r.run("adopt-i18n.mjs", "--branch", merge, "--base", `${merge}^1`, "--onto", onto), before, "is a merge commit and");
    expectRefused(r, r.run("adopt-i18n.mjs", "--branch", branch, "--base", onto, "--onto", onto), before, `${onto} (--base) is not an ancestor of ${branch} (--branch)`);
    const mixed = commitMergeKeepingMonoliths(r);
    expectRefused(r, r.run("adopt-i18n.mjs", "--branch", branch, "--base", r.base, "--onto", mixed), before, `${mixed} (--onto) still has legacy monoliths beside its split areas`);
    r.git("checkout", "-q", "-b", "no-areas", "split");
    r.git("rm", "-q", "-r", "mobile/src/i18n/locales");
    r.git("commit", "-q", "-m", "no split areas");
    const bare = r.git("rev-parse", "HEAD");
    r.git("checkout", "-q", "legacy");
    expectRefused(r, r.run("adopt-i18n.mjs", "--branch", branch, "--base", r.base, "--onto", bare), before, `${bare} (--onto) has no split catalogs to adopt onto`);
  });

  test("refuses a mixed branch whose --base and merge-base diverge", () => {
    const r = repository();
    const { onto } = splitThenLegacyDelta(r);
    r.git("checkout", "-q", "-b", "side", r.base);
    const side = editMonoliths(r, "side", (next, language) => { next["Side key"] = `side ${language}`; });
    r.git("checkout", "-q", "legacy");
    r.git("merge", "-q", "--no-ff", "-X", "theirs", "-m", "merge side", "side");
    commitMergeKeepingMonoliths(r);
    const before = r.git("status", "--porcelain");
    expectRefused(r, r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", side, "--onto", onto), before,
      "do not contain one another, so the reference for the branch's split areas is ambiguous");
  });

  for (const kind of ["add", "remove"]) {
    test(`refuses a key the branch changed in one language only (${kind})`, () => {
      const r = repository();
      const { onto } = splitThenLegacyDelta(r);
      editMonoliths(r, `one-language ${kind}`, (next) => { if (kind === "add") next["Only zh-CN"] = "仅"; else delete next["Sign in"]; }, "zh-CN");
      const before = r.git("status", "--porcelain");
      const result = r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", r.base, "--onto", onto);
      expectRefused(r, result, before, ": key parity differs across zh-CN/zh-TW");
      assert.ok(result.stderr.includes("Every mobile key needs a value in each of zh-CN and zh-TW"), result.stderr);
    });
  }

  test("places a mixed branch's new key by its source file, after the key's area on onto (N11)", () => {
    const r = repository();
    const { onto } = splitThenLegacyDelta(r);
    commitMergeKeepingMonoliths(r);
    write(path.join(r.root, "src/features/settings/Screen.tsx"), 'export const a = [t("Settings key"), t("Change me")];\n');
    editMonoliths(r, "settings copy", (next, language) => { next["Settings key"] = `settings ${language}`; next["Change me"] = `settings change ${language}`; });
    const result = r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", r.base, "--onto", onto);
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(auditCatalogs(r.locales), []);
    for (const language of LANGUAGES) {
      assert.deepEqual(readCatalog(r.locales, "settings", language), { "Settings key": `settings ${language}` });
      assert.equal(readCatalog(r.locales, "account", language)["Change me"], `settings change ${language}`);
    }
  });

  test("cherry-picks a commit that deleted every split area file: its monolith edits land and main keeps every key", () => {
    const r = repository();
    splitThenLegacyDelta(r);
    commitMergeKeepingMonoliths(r);
    r.git("rm", "-q", ...fs.readdirSync(r.locales, { withFileTypes: true }).filter((entry) => entry.isDirectory())
      .flatMap((entry) => LANGUAGES.map((language) => `mobile/src/i18n/locales/${entry.name}/${language}.json`)));
    const deleted = editMonoliths(r, "delete the split areas and change Save", (next, language) => { next.Save = `save deleted ${language}`; });
    r.git("checkout", "-q", "split");
    assert.notEqual(spawnSync("git", ["cherry-pick", deleted], { cwd: r.repo, encoding: "utf8" }).status, 0);
    const result = r.run("adopt-i18n.mjs");
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(auditCatalogs(r.locales), []);
    for (const language of LANGUAGES) {
      assert.deepEqual(mergedCatalog(r, language), {
        Save: `save deleted ${language}`, "Sign in": zh("Sign in", language), "Change me": zh("Change me", language), "Remove me": zh("Remove me", language),
      });
    }
  });

  test("re-lands a reverted commit with cherry-pick, and with explicit refs (N12)", () => {
    const r = repository();
    const { branch } = splitThenLegacyDelta(r);
    r.git("checkout", "-q", "-b", "main3", r.base);
    r.git("merge", "-q", "--no-ff", "-m", "land legacy", branch);
    r.git("revert", "--no-edit", "-m", "1", "HEAD");
    assert.equal(r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", "empty", "--onto", "HEAD").status, 0);
    r.git("add", "-A"); r.git("commit", "-q", "-m", "split main3");
    const main3 = r.git("rev-parse", "HEAD");
    assert.notEqual(spawnSync("git", ["cherry-pick", branch], { cwd: r.repo, encoding: "utf8" }).status, 0);
    const picked = r.run("adopt-i18n.mjs");
    assert.equal(picked.status, 0, picked.stderr);
    expectDeltaAdopted(r);
    r.git("reset", "-q", "--hard"); r.git("clean", "-fdq", "mobile/src"); r.git("cherry-pick", "--quit");
    assert.equal(r.git("rev-parse", "HEAD"), main3);
    const explicit = r.run("adopt-i18n.mjs", "--branch", branch, "--base", `${branch}^`, "--onto", "HEAD");
    assert.equal(explicit.status, 0, explicit.stderr);
    for (const language of LANGUAGES) {
      assert.equal(mergedCatalog(r, language)["Change me"], `changed ${language}`);
      assert.equal(mergedCatalog(r, language).Added, zh("Added", language));
    }
  });

  test("applies a later area edit when the branch's monolith change matches the same change on main (N14)", () => {
    const r = repository();
    splitThenLegacyDelta(r);
    r.git("checkout", "-q", "split");
    for (const language of LANGUAGES) {
      const file = path.join(r.locales, "shared", `${language}.json`);
      write(file, canonicalText({ ...JSON.parse(fs.readFileSync(file, "utf8")), Save: `same ${language}` }));
    }
    r.git("add", "-A"); r.git("commit", "-q", "-m", "main changes Save");
    const onto = r.git("rev-parse", "HEAD");
    r.git("checkout", "-q", "legacy");
    editMonoliths(r, "the branch makes the same change", (next, language) => { next.Save = `same ${language}`; });
    commitMergeKeepingMonoliths(r);
    for (const language of LANGUAGES) {
      const file = path.join(r.locales, "shared", `${language}.json`);
      write(file, canonicalText({ ...JSON.parse(fs.readFileSync(file, "utf8")), Save: `area ${language}` }));
    }
    r.git("add", "-A"); r.git("commit", "-q", "-m", "the branch edits the area");
    const result = r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", r.base, "--onto", onto);
    assert.equal(result.status, 0, result.stderr);
    for (const language of LANGUAGES) assert.equal(mergedCatalog(r, language).Save, `area ${language}`);
  });

  for (const direction of ["forward", "reverse"]) {
    test(`adopts a ${direction} merge (legacy branch checked out, or split branch checked out)`, () => {
      const r = repository();
      splitThenLegacyDelta(r);
      if (direction === "reverse") r.git("checkout", "-q", "split");
      assert.notEqual(spawnSync("git", ["merge", "--no-commit", direction === "reverse" ? "legacy" : "split"], { cwd: r.repo, encoding: "utf8" }).status, 0);
      const result = r.run("adopt-i18n.mjs");
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout.includes("i18n: MERGE_HEAD has the legacy monoliths and HEAD does not, so MERGE_HEAD is adopted onto HEAD"), direction === "reverse", result.stdout);
      expectDeltaAdopted(r);
    });
  }

  test("guides a reverse merge of a branch that kept the monoliths, and the guidance completes it", () => {
    const r = repository();
    mainMovesAfterMixedMerge(r);
    r.git("checkout", "-q", "split");
    spawnSync("git", ["merge", "--no-commit", "legacy"], { cwd: r.repo, encoding: "utf8" });
    const before = r.git("status", "--porcelain");
    const refused = r.run("adopt-i18n.mjs");
    assert.notEqual(refused.status, 0);
    assert.ok(refused.stderr.includes("MERGE_HEAD (--branch) keeps the legacy monoliths beside split areas it inherited from main"), refused.stderr);
    assert.ok(refused.stderr.includes(`  pnpm adopt:i18n --branch MERGE_HEAD --base ${r.base} --onto HEAD`), refused.stderr);
    assert.equal(r.git("status", "--porcelain"), before);
    const guided = runGuidance(r, refused.stderr);
    assert.equal(guided.status, 0, guided.stderr);
    expectNothingReverted(r);
  });

  test("notes merges a rebase drops below its first replayed commit, and still adopts that commit", () => {
    const r = repository();
    splitThenLegacyDelta(r);
    // a1 lands upstream as the same patch, so git skips it and replays only b1, whose parent is the merge M.
    r.git("checkout", "-q", "-b", "a1", r.base);
    write(path.join(r.root, "src/features/auth/A1.tsx"), "export const a1 = 1;\n");
    r.git("add", "-A"); r.git("commit", "-q", "-m", "a1");
    r.git("checkout", "-q", "-b", "upstream", "split");
    r.git("cherry-pick", "a1");
    r.git("checkout", "-q", "-b", "merged", r.base);
    r.git("merge", "-q", "--no-ff", "-m", "M: merge a1", "a1");
    const merge = r.git("rev-parse", "HEAD");
    editMonoliths(r, "b1", (next, language) => { next["Change me"] = `b1 ${language}`; });
    assert.notEqual(spawnSync("git", ["rebase", "upstream"], { cwd: r.repo, encoding: "utf8" }).status, 0);
    assert.equal(r.git("rev-parse", "REBASE_HEAD^"), merge);
    const result = r.run("adopt-i18n.mjs");
    assert.equal(result.status, 0, result.stderr);
    assert.ok(result.stderr.includes(`i18n: notice: this rebase does not replay merge commit ${merge.slice(0, 9)}, so its catalog resolutions are not carried over`), result.stderr);
    assert.ok(result.stderr.includes("then pnpm adopt:i18n;"), result.stderr);
    assert.deepEqual(auditCatalogs(r.locales), []);
    for (const language of LANGUAGES) assert.equal(mergedCatalog(r, language)["Change me"], `b1 ${language}`);
  });

  test("prints no notice for merges main already contains below the fork (plain rebase)", () => {
    const r = repository();
    // Main itself merges a side branch with --no-ff, the legacy branch forks after that merge, then main splits.
    r.git("checkout", "-q", "-b", "side-main", r.base);
    write(path.join(r.root, "src/features/auth/Side.tsx"), "export const side = 1;\n");
    r.git("add", "-A"); r.git("commit", "-q", "-m", "side");
    r.git("checkout", "-q", "-b", "main-with-merge", r.base);
    r.git("merge", "-q", "--no-ff", "-m", "main merges side", "side-main");
    const mainMerge = r.git("rev-parse", "HEAD");
    r.git("checkout", "-q", "-b", "late-legacy", mainMerge);
    editMonoliths(r, "b1", (next, language) => { next["Change me"] = `b1 ${language}`; });
    r.git("checkout", "-q", "-b", "main-with-merge-split", mainMerge);
    assert.equal(r.run("adopt-i18n.mjs", "--branch", "HEAD", "--base", "empty", "--onto", "HEAD").status, 0);
    r.git("add", "-A"); r.git("commit", "-q", "-m", "split");
    r.git("checkout", "-q", "late-legacy");
    assert.notEqual(spawnSync("git", ["rebase", "main-with-merge-split"], { cwd: r.repo, encoding: "utf8" }).status, 0);
    assert.equal(r.git("rev-parse", "REBASE_HEAD^"), mainMerge);
    const result = r.run("adopt-i18n.mjs");
    assert.equal(result.status, 0, result.stderr);
    assert.ok(!result.stderr.includes("i18n: notice"), result.stderr);
    assert.deepEqual(auditCatalogs(r.locales), []);
    for (const language of LANGUAGES) assert.equal(mergedCatalog(r, language)["Change me"], `b1 ${language}`);
  });
});
