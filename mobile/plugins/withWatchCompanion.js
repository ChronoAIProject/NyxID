const { withXcodeProject } = require("expo/config-plugins");

/**
 * The Apple Watch app (`ios/NyxIDWatch`) is a target in the committed
 * NyxIDMobile.xcodeproj. Prebuild rewrites the iPhone target's bundle ID and
 * version for each profile; this keeps the watch target in step, because the
 * App Store rejects a watch app whose bundle ID isn't prefixed by its
 * companion's or whose version differs from it.
 *
 * It also restores the watch's device family. Expo's built-in
 * `withDeviceFamily` sets `TARGETED_DEVICE_FAMILY` to the iPhone/iPad value on
 * every build configuration that has a `PRODUCT_NAME`, the watch target's
 * included. App config plugins register their mods before Expo's built-ins,
 * and later-registered mods run first, so this plugin runs after it.
 */
const WATCH_TARGET = "NyxIDWatch";
/** Apple Watch in `TARGETED_DEVICE_FAMILY`. */
const WATCH_DEVICE_FAMILY = 4;

function unquote(value) {
  return typeof value === "string" ? value.replace(/^"(.*)"$/, "$1") : value;
}

function quote(value) {
  return `"${String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function syncWatchTarget(project, { bundleIdentifier, version, buildNumber }) {
  if (!bundleIdentifier) {
    throw new Error("Apple Watch app: ios.bundleIdentifier is required.");
  }
  const targets = project.pbxNativeTargetSection();
  const target = Object.entries(targets).find(
    ([key, value]) =>
      !key.endsWith("_comment") && unquote(value.name) === WATCH_TARGET,
  )?.[1];
  if (!target) {
    throw new Error(
      `Apple Watch app: the ${WATCH_TARGET} target is missing from NyxIDMobile.xcodeproj. ` +
        "Restore mobile/ios from git instead of regenerating it with --clean.",
    );
  }
  const configurationList =
    project.pbxXCConfigurationList()[target.buildConfigurationList];
  const configurations = project.pbxXCBuildConfigurationSection();
  for (const { value } of configurationList.buildConfigurations) {
    const settings = configurations[value].buildSettings;
    settings.PRODUCT_BUNDLE_IDENTIFIER = quote(`${bundleIdentifier}.watchkitapp`);
    settings.INFOPLIST_KEY_WKCompanionAppBundleIdentifier =
      quote(bundleIdentifier);
    if (version) settings.MARKETING_VERSION = quote(version);
    if (buildNumber) settings.CURRENT_PROJECT_VERSION = quote(buildNumber);
    settings.TARGETED_DEVICE_FAMILY = WATCH_DEVICE_FAMILY;
  }
  return project;
}

module.exports = (config) =>
  withXcodeProject(config, (config) => {
    config.modResults = syncWatchTarget(config.modResults, {
      bundleIdentifier: config.ios?.bundleIdentifier,
      version: config.version,
      buildNumber: config.ios?.buildNumber,
    });
    return config;
  });
module.exports.syncWatchTarget = syncWatchTarget;
module.exports.WATCH_TARGET = WATCH_TARGET;
module.exports.WATCH_DEVICE_FAMILY = WATCH_DEVICE_FAMILY;
