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

test("preserves xterm's three copyright holders on separate lines", () => {
  const copyright = [
    "Copyright (c) 2017-2019, The xterm.js authors (https://github.com/xtermjs/xterm.js)",
    "Copyright (c) 2014-2016, SourceLair Private Company (https://www.sourcelair.com)",
    "Copyright (c) 2012-2013, Christopher Jeffrey (https://github.com/chjj/)",
  ].join("\n");
  const [notice] = reflowTerminalLicenses(
    `@xterm/xterm\n${copyright}\n\nPermission is hereby granted\nfor everyone.`,
  );
  assert.equal(
    notice?.text,
    `${copyright}\n\nPermission is hereby granted for everyone.`,
  );
});

test("recognizes trimmed copyright lines regardless of case while joining wrapped prose", () => {
  const [notice] = reflowTerminalLicenses(
    "@xterm/xterm\nCopyright first holder\n  cOpYrIgHt second holder\nwrapped continuation\n\tCOPYRIGHT third holder",
  );
  assert.equal(
    notice?.text,
    "Copyright first holder\ncOpYrIgHt second holder wrapped continuation\nCOPYRIGHT third holder",
  );
});
