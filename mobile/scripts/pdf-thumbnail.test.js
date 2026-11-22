const test = require("node:test");
const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const { mkdtempSync, rmSync } = require("node:fs");
const { tmpdir } = require("node:os");
const path = require("node:path");

test(
  "native PDF first-page renderer confines reads, output and crop; rejects locked or invalid files",
  { skip: process.platform !== "darwin" },
  (t) => {
    const directory = mkdtempSync(path.join(tmpdir(), "nyx-pdf-swift-"));
    try {
      const source = path.resolve(__dirname, "../modules/nyx-pdf-thumbnail");
      const binary = path.join(directory, "proof");
      execFileSync(
        "swiftc",
        [
          path.join(source, "ios/PdfThumbnailRenderer.swift"),
          path.join(source, "tests/main.swift"),
          "-o",
          binary,
        ],
        { stdio: "pipe" },
      );
      const output = execFileSync(binary, { encoding: "utf8" });
      assert.match(output, /proofs passed/);
      t.diagnostic(output.trim());
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  },
);
