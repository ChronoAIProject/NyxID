const { AndroidConfig, withAndroidManifest } = require("expo/config-plugins");

module.exports = (config) =>
  withAndroidManifest(config, (mod) => {
    const main = AndroidConfig.Manifest.getMainActivityOrThrow(mod.modResults);
    const flags = (main.$["android:configChanges"] || "")
      .split("|")
      .filter(Boolean);
    // RN updates font metrics and relayouts mounted surfaces in onConfigurationChanged.
    if (!flags.includes("fontScale")) flags.push("fontScale");
    main.$["android:configChanges"] = flags.join("|");
    return mod;
  });
