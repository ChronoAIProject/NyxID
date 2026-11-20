import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import test from "node:test";
import appConfig from "../../app.config";
import type { AndroidConfig, ConfigPlugin } from "expo/config-plugins";

const require = createRequire(new URL("../../app.config.ts", import.meta.url));
const { evalModsAsync, withBaseMod } =
  require("expo/config-plugins") as typeof import("expo/config-plugins");

Object.assign(process.env, {
  APP_ENV: "dev",
  DEV_API_BASE_URL: "https://api.example.test/api/v1",
  DEV_FRONTEND_URL: "https://example.test",
  DEV_ANDROID_PACKAGE: "com.example.nyxid",
  DEV_IOS_BUNDLE_ID: "com.example.nyxid",
});

for (const initial of [
  undefined,
  "orientation|screenSize|uiMode",
  "fontScale|orientation",
]) {
  test(`font scale is delivered to MainActivity with existing flags ${initial ?? "(none)"}`, async () => {
    const config = appConfig({
      config: { name: "NyxID", slug: "nyxid" },
      projectRoot: process.cwd(),
      staticConfigPath: null,
      packageJsonPath: null,
    });
    const entry = config.plugins?.find(
      (name) => name === "./plugins/withFontScaleChanges",
    );
    assert.equal(typeof entry, "string");
    const plugin = require(entry as string) as ConfigPlugin;
    const main = {
      $: { "android:name": ".MainActivity", "android:configChanges": initial },
    };
    const other = {
      $: { "android:name": ".Other", "android:configChanges": "keyboard" },
    };
    const manifest = {
      manifest: {
        $: { "xmlns:android": "http://schemas.android.com/apk/res/android" },
        queries: [],
        application: [
          {
            $: { "android:name": ".MainApplication" },
            activity: [other, main],
          },
        ],
      },
    } satisfies AndroidConfig.Manifest.AndroidManifest;
    const mods = withBaseMod(plugin(plugin(config)), {
      platform: "android",
      mod: "manifest",
      isProvider: true,
      action: async (mod) => {
        assert.ok(mod.modRequest.nextMod);
        return mod.modRequest.nextMod({
          ...mod,
          modResults: manifest,
        });
      },
    });
    await evalModsAsync(mods, {
      projectRoot: tmpdir(),
      platforms: ["android"],
    });
    assert.deepEqual(main.$["android:configChanges"]?.split("|"), [
      ...new Set([...(initial?.split("|") ?? []), "fontScale"]),
    ]);
    assert.equal(other.$["android:configChanges"], "keyboard");
  });
}
