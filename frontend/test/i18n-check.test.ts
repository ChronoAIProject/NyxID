// @vitest-environment node
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { FAMILIES, LANGUAGES, canonicalText } from "../scripts/lib/i18n-catalogs.mjs";

const frontend = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const temporaryRoots: string[] = [];
afterEach(() => {
  for (const root of temporaryRoots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

function fixture() {
  const repo = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-i18n-check-"));
  temporaryRoots.push(repo);
  const root = path.join(repo, "frontend");
  const write = (file: string, text: string) => {
    fs.mkdirSync(path.dirname(path.join(root, file)), { recursive: true });
    fs.writeFileSync(path.join(root, file), text);
  };
  for (const file of ["check-i18n.mjs", "sync-i18n.mjs", "lib/i18n-catalogs.mjs", "lib/i18n-portions.mjs"]) {
    write(`scripts/${file}`, fs.readFileSync(path.join(frontend, "scripts", file), "utf8"));
  }
  write("package.json", '{"type":"module"}\n');
  write("../shared/i18n/catalog-core.mjs", fs.readFileSync(path.join(frontend, "../shared/i18n/catalog-core.mjs"), "utf8"));
  fs.symlinkSync(path.join(frontend, "node_modules"), path.join(root, "node_modules"));
  for (const file of ["protocol-examples", "glossary", "simplified-only", "domain-glossary"]) write(`src/i18n/${file}.json`, "[]\n");
  for (const language of LANGUAGES) {
    write(`src/i18n/locales/app/billing/${language}.json`, canonicalText({ Billing: language === "en" ? "Billing" : `Billing ${language}` }));
    for (const family of FAMILIES) write(`src/i18n/locales/${family}-${language}.json`, "{}\n");
  }
  write("src/i18n/locales/app/en-overrides.json", "{}\n");
  write("src/pages/billing.tsx", 'export const Page = () => <p>{t("Billing")}</p>;\n');
  const check = () => spawnSync(process.execPath, [path.join(root, "scripts/check-i18n.mjs")], { cwd: root, encoding: "utf8" });
  const sync = () => spawnSync(process.execPath, [path.join(root, "scripts/sync-i18n.mjs")], { cwd: root, encoding: "utf8" });
  return { root, write, check, sync };
}

describe("check-i18n", () => {
  it("passes a clean portion layout", () => {
    const result = fixture().check();
    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
  });

  it("guards catalog imports in every non-test source while auditing copy only in audited files", () => {
    const f = fixture();
    f.write("src/pages/billing.tsx", 'import en from "@/i18n/locales/app/billing/en.json";\nexport const Page = () => <p>Audited copy</p>;\n');
    f.write("src/lib/fixtures/billing.tsx", 'import errors from "../../i18n/locales/errors-en.json";\nexport const Page = () => <p>Fixture copy</p>;\n');
    f.write("src/dev/preview.tsx", 'export const load = () => import("../i18n/locales/app/en-full");\n');
    f.write("src/pages/billing.test.tsx", 'import full from "../i18n/locales/app/en-full";\n');
    const result = f.check();
    expect(result.status).toBe(1);
    const lines = result.stderr.trim().split("\n").sort();
    expect(lines).toEqual([
      "src/dev/preview.tsx:1: full English catalog is test-only",
      "src/lib/fixtures/billing.tsx:1: catalog imports are restricted to sanctioned i18n modules and tests",
      "src/pages/billing.tsx:1: portion JSON imports are restricted to the catalog barrels and tests",
      "src/pages/billing.tsx:2: untranslated UI literal: Audited copy",
    ]);
  });

  it("reports a remaining legacy catalog import in a test file with its replacement", () => {
    const f = fixture();
    f.write("src/pages/billing.test.tsx", 'import zhCN from "@/i18n/locales/zh-CN.json";\nvi.mock("../i18n/locales/en.json");\n');
    const result = f.check();
    expect(result.status).toBe(1);
    expect(result.stderr.trim().split("\n")).toEqual([
      "src/pages/billing.test.tsx:1: legacy catalog import @/i18n/locales/zh-CN.json; use @/i18n/locales/app/zh-CN (npm run adopt:i18n rewrites it; outside a merge or rebase run npm run adopt:i18n -- --imports-only)",
      "src/pages/billing.test.tsx:2: legacy catalog import ../i18n/locales/en.json; use ../i18n/locales/app/en-full (npm run adopt:i18n rewrites it; outside a merge or rebase run npm run adopt:i18n -- --imports-only)",
    ]);
  });

  it("only warns about an identical cross-area duplicate", () => {
    const f = fixture();
    for (const language of LANGUAGES) f.write(`src/i18n/locales/app/dashboard/${language}.json`, canonicalText({ Billing: language === "en" ? "Billing" : `Billing ${language}` }));
    const result = f.check();
    expect(result.status).toBe(0);
    expect(result.stderr).toBe("i18n: identical duplicate Billing in billing, dashboard; run npm run sync:i18n\n");
  });

  it("flags an emptied area, which sync prunes", () => {
    const f = fixture();
    for (const language of LANGUAGES) f.write(`src/i18n/locales/app/dashboard/${language}.json`, "{}\n");
    const result = f.check();
    expect(result.status).toBe(1);
    expect(result.stderr.trim().split("\n")).toEqual(LANGUAGES.map((language) => `app/dashboard/${language}.json: empty area; run npm run sync:i18n`));
    expect(f.sync().status).toBe(0);
    expect(fs.existsSync(path.join(f.root, "src/i18n/locales/app/dashboard"))).toBe(false);
    expect(fs.existsSync(path.join(f.root, "src/i18n/locales/app/billing"))).toBe(true);
    expect(f.check().status).toBe(0);
  });

  it("flags the pre-split en-overrides.json as a legacy file", () => {
    const f = fixture();
    f.write("src/i18n/locales/en-overrides.json", "{}\n");
    const result = f.check();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("legacy file en-overrides.json is present; run npm run adopt:i18n");
  });
});
