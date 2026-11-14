const test = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const { mkdtempSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const path = require("node:path");
test("Swift real-size native mapping, raw-size decisions and typed discard logs preserve privacy; consent and RN duplicate proofs pass", (t) => {
  const directory = mkdtempSync(path.join(tmpdir(), "nyx-crash-swift-"));
  try {
    const source = path.resolve(__dirname, "../modules/nyx-crash-reporter");
    const binary = path.join(directory, "proof");
    execFileSync(
      "swiftc",
      [
        path.join(source, "ios/CrashDecision.swift"),
        path.join(source, "tests/main.swift"),
        "-o",
        binary,
      ],
      { stdio: "pipe" },
    );
    const output = execFileSync(binary, { encoding: "utf8" });
    assert.match(output, /proofs passed/);
    const outputLines = output.trimEnd().split(/\r?\n/);
    assert.equal(outputLines.at(-1), "Shared pending-raw consume proof passed");
    t.diagnostic(output.trim());
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
