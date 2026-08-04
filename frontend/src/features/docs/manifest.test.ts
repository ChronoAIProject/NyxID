import { describe, it, expect } from "vitest";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import {
  DOCS_TABS,
  DOCS_SHARED,
  allDocPages,
  findDocPage,
  docTabForSlug,
} from "./manifest";

// Source markdown lives at <repo>/docs/site; this file is at
// <repo>/frontend/src/features/docs, so step up four levels.
const SITE_DIR = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../../docs/site",
);

describe("docs manifest", () => {
  const pages = allDocPages();

  it("ships the Phase 1 page set", () => {
    expect(pages.length).toBeGreaterThan(20);
  });

  it("has unique slugs", () => {
    const slugs = pages.map((p) => p.slug);
    expect(new Set(slugs).size).toBe(slugs.length);
  });

  it("gives every page a non-empty title", () => {
    for (const p of pages) expect(p.title.trim().length).toBeGreaterThan(0);
  });

  it("resolves every slug to a real markdown source file", () => {
    for (const p of pages) {
      expect(existsSync(path.join(SITE_DIR, `${p.slug}.md`)), `${p.slug}.md`).toBe(true);
    }
  });

  it("finds a known page and rejects an unknown one", () => {
    expect(findDocPage("shared/concepts/broker-model")?.title).toBeTruthy();
    expect(findDocPage("does/not/exist")).toBeUndefined();
  });

  it("maps slugs to their surface tab", () => {
    expect(docTabForSlug("ai/guides/mcp-proxy")).toBe("ai");
    expect(docTabForSlug("web/getting-started/sign-up")).toBe("web");
    expect(docTabForSlug("cli/getting-started/install")).toBe("cli");
    expect(docTabForSlug("shared/concepts/the-proxy")).toBe("shared");
  });

  it("namespaces every tab's pages under that tab id", () => {
    for (const tab of DOCS_TABS) {
      for (const g of tab.groups) {
        for (const p of g.pages) {
          expect(p.slug.startsWith(`${tab.id}/`), p.slug).toBe(true);
        }
      }
    }
  });

  it("scopes shared groups under shared/", () => {
    for (const g of DOCS_SHARED) {
      for (const p of g.pages) expect(p.slug.startsWith("shared/"), p.slug).toBe(true);
    }
  });
});
