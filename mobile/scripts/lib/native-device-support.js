const PHONE_ORIENTATIONS = [
  "UIInterfaceOrientationPortrait",
  "UIInterfaceOrientationLandscapeLeft",
  "UIInterfaceOrientationLandscapeRight",
];
const IPAD_ORIENTATIONS = [
  "UIInterfaceOrientationPortrait",
  "UIInterfaceOrientationPortraitUpsideDown",
  "UIInterfaceOrientationLandscapeLeft",
  "UIInterfaceOrientationLandscapeRight",
];

function sameOrientations(actual, expected) {
  return (
    Array.isArray(actual) &&
    actual.length === expected.length &&
    expected.every((value) => actual.includes(value))
  );
}

/**
 * TARGETED_DEVICE_FAMILY of every build configuration that sets one, split by
 * platform: watchOS configurations (`SDKROOT = watchos`) belong to the Apple
 * Watch app and never take iPhone/iPad families.
 */
function deviceFamilies(xcodeProject) {
  const ios = [];
  const watch = [];
  for (const [, settings] of xcodeProject.matchAll(
    /buildSettings = \{([\s\S]*?)\n\t*\};/g,
  )) {
    const family = settings.match(/TARGETED_DEVICE_FAMILY\s*=\s*([^;]+);/);
    if (!family) continue;
    const value = family[1].replace(/["\s]/g, "");
    (/\bSDKROOT\s*=\s*watchos;/.test(settings) ? watch : ios).push(value);
  }
  return { ios, watch };
}

/** Check the committed native project as well as the config used by future prebuilds. */
function validateIosDeviceSupport(config, nativeInfoPlist, xcodeProject) {
  const failures = [];
  if (
    config.ios?.supportsTablet !== true ||
    config.ios?.isTabletOnly === true
  ) {
    failures.push("app.config.ts must support both iPhone and iPad");
  }
  if (
    config.ios?.requireFullScreen !== false ||
    nativeInfoPlist.UIRequiresFullScreen !== false
  ) {
    failures.push("app.config.ts and Info.plist must allow iPad multitasking");
  }

  const { ios, watch } = deviceFamilies(xcodeProject);
  if (ios.length === 0 || ios.some((value) => value !== "1,2")) {
    failures.push(
      "every iOS app build configuration must target iPhone and iPad (1,2)",
    );
  }
  // The embedded Apple Watch app (`NyxIDWatch`) is the only watchOS target.
  if (watch.some((value) => value !== "4")) {
    failures.push("every watchOS build configuration must target Apple Watch (4)");
  }

  for (const [source, infoPlist] of [
    ["app.config.ts", config.ios?.infoPlist ?? {}],
    ["Info.plist", nativeInfoPlist],
  ]) {
    if (
      !sameOrientations(
        infoPlist.UISupportedInterfaceOrientations,
        PHONE_ORIENTATIONS,
      )
    ) {
      failures.push(
        `${source} must allow portrait and both iPhone desktop landscape orientations`,
      );
    }
    if (
      !sameOrientations(
        infoPlist["UISupportedInterfaceOrientations~ipad"],
        IPAD_ORIENTATIONS,
      )
    ) {
      failures.push(
        `${source} must allow all four iPad orientations for multitasking`,
      );
    }
    if (
      infoPlist["EXDefaultScreenOrientationMask~ipad"] !==
      "UIInterfaceOrientationMaskAll"
    ) {
      failures.push(
        `${source} must let the iPad window choose its initial orientation`,
      );
    }
  }

  const orientationPlugin = (config.plugins ?? []).find(
    (plugin) =>
      Array.isArray(plugin) && plugin[0] === "expo-screen-orientation",
  );
  if (
    config.orientation !== "portrait" ||
    orientationPlugin?.[1]?.initialOrientation !== "PORTRAIT_UP" ||
    nativeInfoPlist.EXDefaultScreenOrientationMask !==
      "UIInterfaceOrientationMaskPortrait"
  ) {
    failures.push(
      "app.config.ts and Info.plist must keep iPhone's upright portrait default",
    );
  }
  return failures;
}

module.exports = { validateIosDeviceSupport, deviceFamilies };
