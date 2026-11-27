const { AndroidConfig, withAndroidManifest } = require("expo/config-plugins");

module.exports = (config, { binding }) =>
  withAndroidManifest(config, (mod) => {
    const manifest = AndroidConfig.Manifest;
    const app = manifest.getMainApplicationOrThrow(mod.modResults);
    const key = "dev.nyxid.performance.LAUNCH_BINDING";
    manifest.removeMetaDataItemFromMainApplication(app, key);
    if (/^[a-f0-9]{64}$/.test(binding))
      manifest.addMetaDataItemToMainApplication(app, key, binding);
    return mod;
  });
