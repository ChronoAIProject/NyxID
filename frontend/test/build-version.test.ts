import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { assistantFingerprint } from "../scripts/build-version";
import { parseBuildVersion } from "../src/lib/build-version";

let root: string;
function write(file: string, content: string) {
  const absolute = path.join(root, file);
  fs.mkdirSync(path.dirname(absolute), { recursive: true });
  fs.writeFileSync(absolute, content);
}
function fingerprint(buildInputs = "") {
  const graph: Record<string, { importedIds: string[]; dynamicallyImportedIds: string[] }> = {
    "src/pages/assistant.tsx": { importedIds: ["src/lib/shared.ts"], dynamicallyImportedIds: ["src/lib/voice.ts"] },
    "src/main.tsx": { importedIds: ["src/router.tsx", "src/lib/shared.ts"], dynamicallyImportedIds: [] },
    "src/router.tsx": { importedIds: ["src/pages/lazy.ts", "src/lib/shell.ts"], dynamicallyImportedIds: [] },
    "src/pages/lazy.ts": { importedIds: [], dynamicallyImportedIds: ["src/pages/admin.tsx"] },
    "src/lib/shared.ts": { importedIds: ["src/lib/cycle.ts"], dynamicallyImportedIds: [] },
    "src/lib/cycle.ts": { importedIds: ["src/lib/shared.ts"], dynamicallyImportedIds: [] },
  };
  return assistantFingerprint(root, (id) => {
    const dependencies = graph[path.relative(root, id)];
    return dependencies ? {
      importedIds: dependencies.importedIds.map((file) => path.join(root, file)),
      dynamicallyImportedIds: dependencies.dynamicallyImportedIds.map((file) => path.join(root, file)),
    } : null;
  }, buildInputs);
}

beforeEach(() => {
  root = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-assistant-fingerprint-"));
  for (const file of ["src/pages/assistant.tsx", "src/main.tsx", "src/router.tsx", "src/pages/lazy.ts", "src/lib/shell.ts", "src/lib/shared.ts", "src/lib/voice.ts", "src/lib/cycle.ts", "src/pages/admin.tsx", "src/app.css", "src/lib/grok-audio-worklet.js"]) {
    write(file, `original ${file}`);
  }
  write("package-lock.json", JSON.stringify({ version: "1.0.0", packages: { "": { version: "1.0.0" }, "node_modules/react": { version: "19.0.0" } } }));
});
afterEach(() => fs.rmSync(root, { recursive: true, force: true }));

describe("assistant deployment identity", () => {
  it.each(["src/pages/assistant.tsx", "src/lib/shared.ts", "src/lib/voice.ts", "src/lib/shell.ts", "src/app.css", "src/router.tsx", "src/lib/grok-audio-worklet.js"])("changes for assistant, dependencies and shell inputs: %s", (file) => {
    const before = fingerprint();
    write(file, "changed");
    expect(fingerprint()).not.toBe(before);
  });

  it("fails closed when the module graph or a source is unavailable", () => {
    expect(() => assistantFingerprint(root, () => null)).toThrow("module graph");
    fs.rmSync(path.join(root, "src/lib/shared.ts"));
    expect(() => fingerprint()).toThrow("could not be read");
  });

  it("stays unchanged for an unrelated route edit or project version bump", () => {
    const before = fingerprint();
    write("src/pages/admin.tsx", "changed admin implementation");
    write("package-lock.json", JSON.stringify({ version: "2.0.0", packages: { "": { version: "2.0.0" }, "node_modules/react": { version: "19.0.0" } } }));
    expect(fingerprint()).toBe(before);
  });

  it("changes for dependency upgrades and public build configuration", () => {
    const before = fingerprint();
    expect(fingerprint("different public build configuration")).not.toBe(before);
    write("package-lock.json", JSON.stringify({ packages: { "node_modules/react": { version: "20.0.0" } } }));
    expect(fingerprint()).not.toBe(before);
  });

  it("uses relative source identities across different worktrees", () => {
    const firstRoot = root;
    const secondRoot = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-assistant-fingerprint-copy-"));
    try {
      const before = fingerprint();
      fs.cpSync(firstRoot, secondRoot, { recursive: true });
      root = secondRoot;
      expect(fingerprint()).toBe(before);
    } finally {
      root = firstRoot;
      fs.rmSync(secondRoot, { recursive: true, force: true });
    }
  });

  it("accepts the real-sized initial graph and refuses external asset paths", () => {
    const version = { buildId: "build", assets: Array.from({ length: 127 }, (_, i) => `assets/index.es-${i}.js`) };
    expect(parseBuildVersion(version)).not.toBeNull();
    expect(parseBuildVersion({ ...version, assets: ["../outside.js"] })).toBeNull();
    expect(parseBuildVersion({ ...version, assets: ["https://example.com/app.js"] })).toBeNull();
  });
});
