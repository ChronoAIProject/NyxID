const assert = require("node:assert/strict");
const test = require("node:test");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { collect } = require("./crash-artifacts");

for (const missing of ["bundle", "map", "dsyms", "uuids"]) {
  for (const configured of [false, true]) {
    test(`missing ${missing} ${configured ? "fails configured symbol upload" : "warns and continues the store build"}`, () => {
      const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nyx-crash-artifacts-"));
      try {
        const info = { directory, platform: "ios", app_version: "1", app_build: "2" };
        const bundle = path.join(directory, "source.js"), map = `${bundle}.map`;
        const dsyms = path.join(directory, "source-dsyms");
        if (missing !== "bundle") fs.writeFileSync(bundle, "hermes-bytecode");
        if (missing !== "map") fs.writeFileSync(map, "{}");
        if (missing !== "dsyms") fs.mkdirSync(dsyms);
        const warnings = [];
        const environment = configured ? {
          CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN: "secret",
          CRASH_ARTIFACTS_SENTRY_ORG: "org",
          CRASH_ARTIFACTS_SENTRY_PROJECT_MOBILE: "project",
        } : {};
        const run = () => collect(info, bundle, map, dsyms, environment,
          () => assert.fail("missing artifacts must not upload"), (line) => warnings.push(line));
        if (configured) assert.throws(run, /Crash (bundle or composed source map|dSYMs|dSYM UUIDs) missing/);
        else {
          assert.doesNotThrow(run);
          assert.deepEqual(warnings, ["Crash artifacts unavailable; symbol upload is not configured. Continuing store build."]);
        }
      } finally { fs.rmSync(directory, { recursive: true, force: true }); }
    });
  }
}

test("configured uploader failure remains fatal after complete Android artifacts", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nyx-crash-upload-"));
  try {
    const bundle = path.join(directory, "source.js"), map = `${bundle}.map`;
    fs.writeFileSync(bundle, "bundle"); fs.writeFileSync(map, "{}");
    assert.throws(() => collect({ directory, platform: "android" }, bundle, map, undefined,
      { CRASH_ARTIFACTS_SENTRY_AUTH_TOKEN: "secret" }, () => { throw Error("upload failed"); }), /upload failed/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

for (const platform of ["ios", "android"]) {
  test(`complete ${platform} artifacts without upload secrets never invoke the uploader`, () => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nyx-crash-local-"));
    try {
      const bundle = path.join(directory, "source.js"), map = `${bundle}.map`;
      fs.writeFileSync(bundle, "bundle");
      fs.writeFileSync(map, "{}");
      const dsyms = platform === "ios" ? path.join(directory, "source-dsyms") : undefined;
      if (dsyms) fs.mkdirSync(path.join(dsyms, "NyxID.dSYM"), { recursive: true });
      collect({ directory, platform }, bundle, map, dsyms, {}, (command) => {
        assert.equal(command, "dwarfdump", "only local UUID discovery is allowed");
        return "UUID: 01234567-89AB-CDEF-0123-456789ABCDEF (arm64) NyxID";
      }, () => assert.fail("complete artifacts must not warn"));
      assert.ok(fs.existsSync(path.join(directory, "manifest.json")));
    } finally { fs.rmSync(directory, { recursive: true, force: true }); }
  });
}
