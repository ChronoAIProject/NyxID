import { readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ASSISTANT_DIR = path.dirname(fileURLToPath(import.meta.url));
const FORBIDDEN_MARKERS = ["}/stream", "}/approve", "}/stop"] as const;

function assistantRuntimeFiles(root: string): string[] {
  const entries = readdirSync(root).sort();
  const files: string[] = [];
  for (const entry of entries) {
    const absolute = path.join(root, entry);
    const stats = statSync(absolute);
    if (stats.isDirectory()) {
      files.push(...assistantRuntimeFiles(absolute));
      continue;
    }
    if (!absolute.endsWith(".ts")) continue;
    if (absolute.endsWith(".test.ts")) continue;
    files.push(absolute);
  }
  return files;
}

describe("assistant canonical command guard", () => {
  it("keeps runtime assistant sources free of scoped command URL templates", () => {
    const offenders = assistantRuntimeFiles(ASSISTANT_DIR).flatMap((file) => {
      const source = readFileSync(file, "utf8");
      return FORBIDDEN_MARKERS.filter((marker) => source.includes(marker)).map(
        (marker) => `${path.relative(ASSISTANT_DIR, file)}:${marker}`,
      );
    });

    expect(offenders).toEqual([]);
  });
});
