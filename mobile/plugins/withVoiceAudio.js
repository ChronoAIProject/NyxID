const { withAppDelegate } = require("expo/config-plugins");

const CONFIGURATION = `    // Keep WebRTC voice processing and headset routing; use the loudspeaker
    // when no external audio route is selected, including after interruptions.
    let voiceConfiguration = RTCAudioSessionConfiguration.webRTC()
    voiceConfiguration.categoryOptions.insert(.defaultToSpeaker)
    RTCAudioSessionConfiguration.setWebRTC(voiceConfiguration)
`;

function configureVoiceAudio(contents) {
  if (!/^import WebRTC$/m.test(contents)) {
    contents = contents.replace(
      /^import ReactAppDependencyProvider$/m,
      "$&\nimport WebRTC",
    );
  }
  if (
    !contents.includes(
      "RTCAudioSessionConfiguration.setWebRTC(voiceConfiguration)",
    )
  ) {
    const anchor = "    let delegate = ReactNativeDelegate()";
    if (!contents.includes(anchor))
      throw new Error(
        "Voice audio configuration: AppDelegate startup anchor is missing.",
      );
    contents = contents.replace(anchor, `${CONFIGURATION}\n${anchor}`);
  }
  return contents;
}

module.exports = (config) =>
  withAppDelegate(config, (config) => {
    if (config.modResults.language !== "swift")
      throw new Error(
        "Voice audio configuration requires the Swift AppDelegate.",
      );
    config.modResults.contents = configureVoiceAudio(
      config.modResults.contents,
    );
    return config;
  });
module.exports.configureVoiceAudio = configureVoiceAudio;
