import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";

function appConfig(appEnv: "dev" | "prod") {
  const filename = fileURLToPath(
    new URL("../../app.config.ts", import.meta.url),
  );
  const exports: Record<string, any> = {};
  const output = ts.transpileModule(readFileSync(filename, "utf8"), {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText;
  runInNewContext(
    output,
    {
      exports,
      process: { env: { APP_ENV: appEnv } },
      require: (name: string) => {
        if (name === "./scripts/lib/load-env")
          return {
            loadEnv: () => ({}),
            resolveProfile: () => ({
              androidPackage: "dev.nyxid.test",
              androidVersionCode: "1",
            }),
            appIdentity: () => ({
              name: "NyxID Test",
              slug: "nyxid-test",
              scheme: "nyxid-test",
              version: "1.0.0",
            }),
          };
        if (name === "./scripts/lib/native-links")
          return {
            buildNativeLinks: () => ({
              associatedDomains: [],
              androidIntentFilters: [],
            }),
          };
        throw new Error(`Unexpected config dependency: ${name}`);
      },
    },
    { filename },
  );
  return exports.default({ config: {} });
}

for (const profile of ["dev", "prod"] as const) {
  test(`${profile} Android config declares both permissions Chromium needs for voice capture`, () => {
    const { android } = appConfig(profile);
    for (const permission of [
      "android.permission.RECORD_AUDIO",
      "android.permission.MODIFY_AUDIO_SETTINGS",
    ]) {
      assert.ok(
        android.permissions.includes(permission),
        `${permission} is declared`,
      );
      assert.ok(
        !android.blockedPermissions.includes(permission),
        `${permission} is not blocked`,
      );
    }
  });
}

test("Save Image has the Photos add-only usage description in resolved config and the tracked iOS plist", () => {
  const description =
    "Allow NyxID to save images you choose from chats to your photo library.";
  for (const profile of ["dev", "prod"] as const) {
    assert.equal(
      appConfig(profile).ios.infoPlist.NSPhotoLibraryAddUsageDescription,
      description,
    );
  }
  const plist = readFileSync(
    new URL("../../ios/NyxIDMobile/Info.plist", import.meta.url),
    "utf8",
  );
  assert.ok(
    plist.includes(
      `<key>NSPhotoLibraryAddUsageDescription</key>\n    <string>${description}</string>`,
    ),
  );
});
