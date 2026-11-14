import assert from "node:assert/strict";
import test from "node:test";
import { reflowTerminalLicenses } from "./terminalLicenseText";

test("reflows wrapped license paragraphs while preserving package sections", () => {
  const notices = reflowTerminalLicenses(
    "@xterm/xterm\nCopyright line\n\nPermission is hereby granted\nfor everyone.\n\n@xterm/addon-fit\nCopyright line",
  );
  assert.deepEqual(
    notices,
    [
      {
        packageName: "@xterm/xterm",
        text: "Copyright line\n\nPermission is hereby granted for everyone.",
      },
      { packageName: "@xterm/addon-fit", text: "Copyright line" },
    ],
  );
});
