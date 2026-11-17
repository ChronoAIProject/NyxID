import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { URL } from "node:url";
import type { ConfigPlugin } from "expo/config-plugins";
import appConfig from "../../app.config";

const require = createRequire(new URL("../../app.config.ts", import.meta.url));
const { evalModsAsync, withBaseMod } =
  require("expo/config-plugins") as typeof import("expo/config-plugins");

function fixture(profile: "dev" | "prod", file: string) {
  const previous = { ...process.env };
  const prefix = profile.toUpperCase();
  Object.assign(process.env, {
    APP_ENV: profile,
    [`${prefix}_API_BASE_URL`]: "https://api.example.test/api/v1",
    [`${prefix}_FRONTEND_URL`]: "https://example.test",
    [`${prefix}_IOS_BUNDLE_ID`]: "com.example.fork",
    [`${prefix}_ANDROID_PACKAGE`]: "com.example.fork",
    [`${prefix}_GOOGLE_SERVICES_FILE`]: file,
    PROD_NEW_API_BASE_URL: "",
    PROD_NEW_FRONTEND_URL: "",
    PROD_NEW_LEGAL_BASE_URL: "",
  });
  try {
    const config = appConfig({
      config: { name: "NyxID Test", slug: "nyxid-test" },
      projectRoot: process.cwd(),
      staticConfigPath: null,
      packageJsonPath: null,
    });
    const plugin = config.plugins?.find(
      (entry) =>
        Array.isArray(entry) &&
        entry[0] === "./plugins/withAndroidGoogleServicesValidation",
    );
    assert.ok(Array.isArray(plugin));
    const modulePath = plugin[0];
    assert.ok(modulePath);
    const apply = require(modulePath) as ConfigPlugin;
    return withBaseMod(apply(config, plugin[1]), {
      platform: "android",
      mod: "manifest",
      isProvider: true,
      // Supply an in-memory manifest through Expo's real mod chain.
      action: (mod) => {
        assert.ok(mod.modRequest.nextMod);
        return mod.modRequest.nextMod({
          ...mod,
          modResults: {
            manifest: { $: { package: config.android!.package! } },
          },
        });
      },
    });
  } finally {
    for (const key of Object.keys(process.env)) {
      if (!(key in previous)) delete process.env[key];
    }
    Object.assign(process.env, previous);
  }
}

for (const profile of ["dev", "prod"] as const) {
  for (const kind of ["missing", "mismatched"] as const) {
    test(`${profile} iOS config and mods ignore a ${kind} Android Firebase file`, async (t) => {
      const directory = mkdtempSync(join(tmpdir(), "nyxid-ios-firebase-"));
      t.after(() => rmSync(directory, { recursive: true, force: true }));
      const file = join(directory, "google-services.json");
      if (kind === "mismatched") writeFileSync(file, '{"client":[]}');
      const config = await evalModsAsync(fixture(profile, file), {
        projectRoot: directory,
        platforms: ["ios"],
      });
      assert.equal(config.ios?.bundleIdentifier, "com.example.fork");
      await assert.rejects(
        evalModsAsync(fixture(profile, file), {
          projectRoot: directory,
          platforms: ["android"],
        }),
        kind === "missing"
          ? /Android Google Services file not found/
          : /does not contain a client for com\.example\.fork/,
      );
    });
  }

  test(`${profile} Android mods accept the Firebase client matching the package`, async (t) => {
    const directory = mkdtempSync(join(tmpdir(), "nyxid-android-firebase-"));
    t.after(() => rmSync(directory, { recursive: true, force: true }));
    const file = join(directory, "google-services.json");
    writeFileSync(
      file,
      JSON.stringify({
        client: [
          {
            client_info: {
              android_client_info: { package_name: "com.example.fork" },
            },
          },
        ],
      }),
    );
    const config = await evalModsAsync(fixture(profile, file), {
      projectRoot: directory,
      platforms: ["android"],
    });
    assert.equal(config.android?.googleServicesFile, file);
    assert.equal(config.android?.package, "com.example.fork");
  });
}

test("Android mods allow development without remote FCM configured", async () => {
  const config = await evalModsAsync(fixture("dev", ""), {
    projectRoot: tmpdir(),
    platforms: ["android"],
  });
  assert.equal(config.android?.googleServicesFile, undefined);
});
