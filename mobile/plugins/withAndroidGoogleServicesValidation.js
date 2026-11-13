const { withAndroidManifest } = require("expo/config-plugins");
const {
  validateAndroidGoogleServicesFile,
} = require("../scripts/lib/load-env");

module.exports = (config, { file, androidPackage }) =>
  withAndroidManifest(config, (config) => {
    if (file) validateAndroidGoogleServicesFile(file, androidPackage);
    return config;
  });
