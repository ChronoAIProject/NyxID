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

  const deviceFamilies = [
    ...xcodeProject.matchAll(/TARGETED_DEVICE_FAMILY\s*=\s*([^;]+);/g),
  ];
  if (
    deviceFamilies.length === 0 ||
    deviceFamilies.some(([, value]) => value.replace(/["\s]/g, "") !== "1,2")
  ) {
    failures.push(
      "every iOS app build configuration must target iPhone and iPad (1,2)",
    );
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

module.exports = { validateIosDeviceSupport };
