import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import test from "node:test";

test("offline terminal bundle and MIT notices match the installed locked dependencies", () => {
  const generator = fileURLToPath(
    new URL("../../../scripts/generate-terminal-assets.js", import.meta.url),
  );
  const result = spawnSync(process.execPath, [generator, "--check"], {
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.error?.message);
});
