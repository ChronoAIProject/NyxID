import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import plist from "@expo/plist";

const require = createRequire(import.meta.url);
const xcode = require(
  require.resolve("xcode", {
    paths: [require.resolve("expo/config-plugins")],
  }),
) as { project(path: string): XcodeProject };
const withWatchCompanion = require("../../../plugins/withWatchCompanion") as {
  (config: ModConfig): ModConfig;
  syncWatchTarget(
    project: XcodeProject,
    values: { bundleIdentifier?: string; version?: string; buildNumber?: string },
  ): XcodeProject;
  WATCH_TARGET: string;
  WATCH_DEVICE_FAMILY: number;
};
const { syncWatchTarget, WATCH_TARGET, WATCH_DEVICE_FAMILY } = withWatchCompanion;
const { IOSConfig } = require("expo/config-plugins") as {
  IOSConfig: {
    DeviceFamily: {
      setDeviceFamily(config: ModConfig, options: { project: XcodeProject }): XcodeProject;
      withDeviceFamily(config: ModConfig): ModConfig;
    };
  };
};

type ModConfig = {
  name: string;
  slug: string;
  version?: string;
  ios?: { bundleIdentifier?: string; buildNumber?: string; supportsTablet?: boolean };
  mods?: { ios?: { xcodeproj?: (config: Record<string, unknown>) => Promise<{ modResults: XcodeProject }> } };
};

type BuildSettings = Record<string, string | string[] | undefined>;
type XcodeProject = {
  parseSync(): XcodeProject;
  pbxNativeTargetSection(): Record<string, { name?: string; buildConfigurationList: string; productType?: string }>;
  pbxXCConfigurationList(): Record<string, { buildConfigurations: { value: string }[] }>;
  pbxXCBuildConfigurationSection(): Record<string, { buildSettings: BuildSettings }>;
  getFirstTarget(): { firstTarget: { name: string } };
  writeSync(): string;
};

const projectPath = fileURLToPath(
  new URL("../../../ios/NyxIDMobile.xcodeproj/project.pbxproj", import.meta.url),
);
const infoPlist = plist.parse(
  readFileSync(
    fileURLToPath(new URL("../../../ios/NyxIDMobile/Info.plist", import.meta.url)),
    "utf8",
  ),
) as Record<string, string>;

// The pbxproj parser yields numbers for bare numeric values (`45`).
const unquote = (value: unknown) =>
  value === undefined ? undefined : String(value).replace(/^"(.*)"$/, "$1");

function load() {
  return xcode.project(projectPath).parseSync();
}

function settingsOf(project: XcodeProject, name: string): BuildSettings[] {
  const target = Object.entries(project.pbxNativeTargetSection()).find(
    ([key, value]) => !key.endsWith("_comment") && unquote(value.name) === name,
  )?.[1];
  assert.ok(target, `${name} target exists`);
  const list = project.pbxXCConfigurationList()[target.buildConfigurationList]!;
  const section = project.pbxXCBuildConfigurationSection();
  return list.buildConfigurations.map(({ value }) => section[value]!.buildSettings);
}

test("the committed watch app is a companion of the committed iPhone app", () => {
  const project = load();
  // Prebuild rewrites the bundle ID of the first target only.
  assert.equal(unquote(project.getFirstTarget().firstTarget.name), "NyxIDMobile");
  const app = settingsOf(project, "NyxIDMobile");
  const watch = settingsOf(project, WATCH_TARGET);
  assert.equal(watch.length, app.length);
  for (const settings of watch) {
    const companion = unquote(app[0]!.PRODUCT_BUNDLE_IDENTIFIER);
    assert.equal(unquote(settings.PRODUCT_BUNDLE_IDENTIFIER), `${companion}.watchkitapp`);
    assert.equal(unquote(settings.INFOPLIST_KEY_WKCompanionAppBundleIdentifier), companion);
    assert.equal(unquote(settings.SDKROOT), "watchos");
    assert.equal(unquote(settings.MARKETING_VERSION), infoPlist.CFBundleShortVersionString);
    assert.equal(unquote(settings.CURRENT_PROJECT_VERSION), infoPlist.CFBundleVersion);
  }
  // The iPhone app embeds the watch app.
  const source = readFileSync(projectPath, "utf8");
  assert.match(source, /dstPath = "\$\(CONTENTS_FOLDER_PATH\)\/Watch";/);
  assert.match(source, /NyxIDWatch\.app in Embed Watch Content/);
});

test("prebuild keeps the watch app's identity and version in step with each profile", () => {
  const project = syncWatchTarget(load(), {
    bundleIdentifier: "dev.nyxid.mobile.example",
    version: "2.3.4",
    buildNumber: "88",
  });
  for (const settings of settingsOf(project, WATCH_TARGET)) {
    assert.equal(unquote(settings.PRODUCT_BUNDLE_IDENTIFIER), "dev.nyxid.mobile.example.watchkitapp");
    assert.equal(unquote(settings.INFOPLIST_KEY_WKCompanionAppBundleIdentifier), "dev.nyxid.mobile.example");
    assert.equal(unquote(settings.MARKETING_VERSION), "2.3.4");
    assert.equal(unquote(settings.CURRENT_PROJECT_VERSION), "88");
    assert.equal(unquote(settings.TARGETED_DEVICE_FAMILY), "4");
  }
  // The iPhone target is untouched by this plugin.
  assert.equal(
    unquote(settingsOf(project, "NyxIDMobile")[0]!.PRODUCT_BUNDLE_IDENTIFIER),
    unquote(settingsOf(load(), "NyxIDMobile")[0]!.PRODUCT_BUNDLE_IDENTIFIER),
  );
  // Writing the project back is stable.
  assert.ok(project.writeSync().includes("dev.nyxid.mobile.example.watchkitapp"));
});

const appConfig = (): ModConfig => ({
  name: "NyxID",
  slug: "nyxid",
  version: "2.3.4",
  ios: { bundleIdentifier: "dev.nyxid.mobile.example", buildNumber: "88", supportsTablet: true },
});

test("the committed watch app targets Apple Watch only", () => {
  assert.equal(WATCH_DEVICE_FAMILY, 4);
  for (const settings of settingsOf(load(), WATCH_TARGET)) {
    assert.equal(unquote(settings.TARGETED_DEVICE_FAMILY), "4");
  }
});

test("Expo's device-family step cannot turn the watch app into an iPhone app", () => {
  const project = IOSConfig.DeviceFamily.setDeviceFamily(appConfig(), { project: load() });
  // Expo writes the iPhone/iPad family on every target that names a product.
  for (const settings of settingsOf(project, WATCH_TARGET)) {
    assert.equal(unquote(settings.TARGETED_DEVICE_FAMILY), "1,2");
  }
  syncWatchTarget(project, { bundleIdentifier: "dev.nyxid.mobile.example" });
  for (const settings of settingsOf(project, WATCH_TARGET)) {
    assert.equal(unquote(settings.TARGETED_DEVICE_FAMILY), "4");
  }
  for (const settings of settingsOf(project, "NyxIDMobile")) {
    assert.equal(unquote(settings.TARGETED_DEVICE_FAMILY), "1,2");
  }
});

test("in prebuild's order, the watch plugin runs after Expo's device-family step", async () => {
  // getPrebuildConfig applies app.config plugins first, then Expo's built-in
  // iOS plugins (withIosExpoPlugins includes withDeviceFamily). Mods registered
  // later run first, so the watch plugin gets the last word.
  const config = IOSConfig.DeviceFamily.withDeviceFamily(withWatchCompanion(appConfig()));
  const xcodeproj = config.mods?.ios?.xcodeproj;
  assert.ok(xcodeproj, "both plugins registered an Xcode project mod");
  const { modResults } = await xcodeproj({
    ...config,
    modResults: load(),
    modRequest: { platform: "ios", modName: "xcodeproj", introspect: false },
  });
  for (const settings of settingsOf(modResults, WATCH_TARGET)) {
    assert.equal(unquote(settings.TARGETED_DEVICE_FAMILY), "4");
    assert.equal(unquote(settings.PRODUCT_BUNDLE_IDENTIFIER), "dev.nyxid.mobile.example.watchkitapp");
  }
  for (const settings of settingsOf(modResults, "NyxIDMobile")) {
    assert.equal(unquote(settings.TARGETED_DEVICE_FAMILY), "1,2");
  }
});

test("a project regenerated without the watch target fails loudly", () => {
  const project = load();
  const targets = project.pbxNativeTargetSection();
  for (const [key, value] of Object.entries(targets)) {
    if (!key.endsWith("_comment") && unquote(value.name) === WATCH_TARGET) delete targets[key];
  }
  assert.throws(
    () => syncWatchTarget(project, { bundleIdentifier: "dev.nyxid.mobile.example" }),
    /NyxIDWatch target is missing/,
  );
});
