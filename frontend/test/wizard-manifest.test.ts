import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, expect, it } from "vitest";
import { wizardSourceFiles } from "../vite-plugins/wizard-manifest";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

it("includes the recursive local CSS closure in the wizard freshness manifest", () => {
  const root = mkdtempSync(path.join(tmpdir(), "nyx-wizard-manifest-"));
  roots.push(root);
  mkdirSync(path.join(root, "src"));
  writeFileSync(path.join(root, "src/app.css"), '@import "tailwindcss";\n/* @import "./ignored.css"; */\n@import"./typography.css";');
  writeFileSync(path.join(root, "src/ignored.css"), "");
  writeFileSync(path.join(root, "src/typography.css"), '@import url("./lines.css");');
  writeFileSync(path.join(root, "src/lines.css"), '@import "./app.css";');
  expect(wizardSourceFiles([
    path.join(root, "src/app.css"),
    path.join(root, "src/entry.tsx"),
    path.join(root, "node_modules/ignored.js"),
    path.join(root, "\0virtual.css"),
    path.join(path.dirname(root), "outside.ts"),
  ], root)).toEqual(["src/app.css", "src/entry.tsx", "src/lines.css", "src/typography.css"]);
});

it("pins the shared typography stylesheet used by the actual wizard", () => {
  const root = path.resolve("..");
  expect(wizardSourceFiles([path.join(root, "frontend/src/app.css")], root))
    .toContain("frontend/src/typography.css");
});
