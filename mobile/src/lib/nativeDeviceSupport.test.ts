import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { URL } from "node:url";
import type { ExpoConfig } from "expo/config";
import appConfig from "../../app.config";

const require = createRequire(import.meta.url);
const plist = require("@expo/plist").default as {
  parse(contents: string): Record<string, unknown>;
};
const { IOSConfig } = require("expo/config-plugins") as {
  IOSConfig: {
    DeviceFamily: { getDeviceFamilies(config: ExpoConfig): number[] };
    RequiresFullScreen: {
      setRequiresFullScreen(
        config: ExpoConfig,
        infoPlist: Record<string, unknown>,
      ): Record<string, unknown>;
    };
  };
};
const { setInitialOrientation } =
  require("expo-screen-orientation/plugin/build/withScreenOrientation") as {
    setInitialOrientation(
      config: { initialOrientation: string },
      infoPlist: Record<string, unknown>,
    ): Record<string, unknown>;
  };
const { validateIosDeviceSupport } =
  require("../../scripts/lib/native-device-support") as {
    validateIosDeviceSupport(
      config: ExpoConfig,
      nativeInfoPlist: Record<string, unknown>,
      xcodeProject: string,
    ): string[];
  };
const nativeInfoPlist = plist.parse(
  readFileSync(
    new URL("../../ios/NyxIDMobile/Info.plist", import.meta.url),
    "utf8",
  ),
);
const xcodeProject = readFileSync(
  new URL("../../ios/NyxIDMobile.xcodeproj/project.pbxproj", import.meta.url),
  "utf8",
);

function fixture() {
  const previous = { ...process.env };
  Object.assign(process.env, {
    APP_ENV: "dev",
    DEV_API_BASE_URL: "http://localhost:3001/api/v1",
    DEV_FRONTEND_URL: "http://localhost:3000",
    DEV_IOS_BUNDLE_ID: "dev.nyxid.nativecheck",
    DEV_ANDROID_PACKAGE: "dev.nyxid.nativecheck",
    DEV_IOS_BUILD_NUMBER: "1",
  });
  try {
    const config = appConfig({
      config: { name: "NyxID Mobile", slug: "nyxid-mobile" },
      projectRoot: process.cwd(),
      staticConfigPath: null,
      packageJsonPath: null,
    });
    return {
      config,
      native: structuredClone(nativeInfoPlist),
      project: xcodeProject,
    };
  } finally {
    for (const key of Object.keys(process.env)) {
      if (!(key in previous)) delete process.env[key];
    }
    Object.assign(process.env, previous);
  }
}

test("Expo configuration and committed Debug/Release targets support iPad without changing iPhone defaults", () => {
  const { config, native, project } = fixture();
  assert.deepEqual(validateIosDeviceSupport(config, native, project), []);
  assert.equal([...project.matchAll(/TARGETED_DEVICE_FAMILY\s*=/g)].length, 2);
});

test("installed Expo native transforms generate the committed universal device and orientation settings", () => {
  const { config, native } = fixture();
  assert.deepEqual(IOSConfig.DeviceFamily.getDeviceFamilies(config), [1, 2]);
  const orientationPlugin = config.plugins!.find(
    (plugin) =>
      Array.isArray(plugin) && plugin[0] === "expo-screen-orientation",
  ) as [string, { initialOrientation: string }];
  const generated = setInitialOrientation(
    orientationPlugin[1],
    IOSConfig.RequiresFullScreen.setRequiresFullScreen(config, {
      ...config.ios!.infoPlist,
    }),
  );
  for (const key of [
    "UIRequiresFullScreen",
    "UISupportedInterfaceOrientations~ipad",
    "EXDefaultScreenOrientationMask",
    "EXDefaultScreenOrientationMask~ipad",
  ]) {
    assert.deepEqual(
      generated[key],
      native[key],
      `${key} must survive Expo native generation`,
    );
  }
});

test("native sync detects the iPhone-only target that causes iPad compatibility letterboxing", () => {
  const { config, native, project } = fixture();
  const phoneOnly = project.replace(
    'TARGETED_DEVICE_FAMILY = "1,2";',
    "TARGETED_DEVICE_FAMILY = 1;",
  );
  assert.match(
    validateIosDeviceSupport(config, native, phoneOnly).join("\n"),
    /every iOS app build configuration/,
  );
  config.ios!.supportsTablet = false;
  assert.match(
    validateIosDeviceSupport(config, native, project).join("\n"),
    /must support both iPhone and iPad/,
  );
});

test("native sync rejects multitasking and iPad orientation drift in either configuration source", () => {
  for (const target of ["config", "native"] as const) {
    const { config, native, project } = fixture();
    const info = target === "config" ? config.ios!.infoPlist! : native;
    info["UISupportedInterfaceOrientations~ipad"] = [
      "UIInterfaceOrientationPortrait",
    ];
    info["EXDefaultScreenOrientationMask~ipad"] =
      "UIInterfaceOrientationMaskPortrait";
    const failures = validateIosDeviceSupport(config, native, project).join(
      "\n",
    );
    assert.match(failures, /all four iPad orientations/);
    assert.match(failures, /iPad window choose its initial orientation/);
  }
  const { config, native, project } = fixture();
  native.UIRequiresFullScreen = true;
  assert.match(
    validateIosDeviceSupport(config, native, project).join("\n"),
    /allow iPad multitasking/,
  );
});

test("native sync catches an accidental change to the existing iPhone portrait default", () => {
  const { config, native, project } = fixture();
  native.EXDefaultScreenOrientationMask = "UIInterfaceOrientationMaskAll";
  assert.match(
    validateIosDeviceSupport(config, native, project).join("\n"),
    /iPhone's upright portrait default/,
  );
});
