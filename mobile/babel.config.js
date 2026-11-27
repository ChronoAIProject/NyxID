module.exports = function (api) {
  // jest-expo also calls itself "metro". Only Expo's real transformer sets isNodeModule.
  const nativeApp = api.caller(
    (caller) =>
      caller?.name === "metro" &&
      caller.isNodeModule === false &&
      (caller.platform === "android" || caller.platform === "ios"),
  );
  return {
    presets: ["babel-preset-expo"],
    plugins: [
      ...(nativeApp ? [require("./scripts/babel-lucide-icons").lucideIcons] : []),
      "react-native-reanimated/plugin",
    ],
  };
};

