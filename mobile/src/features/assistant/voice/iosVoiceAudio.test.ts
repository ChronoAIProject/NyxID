import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { URL } from "node:url";
import test from "node:test";

const require = createRequire(import.meta.url);
const { configureVoiceAudio } =
  require("../../../../plugins/withVoiceAudio") as {
    configureVoiceAudio(source: string): string;
  };
const appDelegate = readFileSync(
  new URL("../../../../ios/NyxIDMobile/AppDelegate.swift", import.meta.url),
  "utf8",
);

test("Expo regeneration retains speaker defaults before WebRTC creates its audio device", () => {
  const fresh = appDelegate
    .replace(/^import WebRTC\n/m, "")
    .replace(
      /    \/\/ Keep WebRTC[^]*?RTCAudioSessionConfiguration.setWebRTC\(voiceConfiguration\)\n\n/,
      "",
    );
  const generated = configureVoiceAudio(fresh);
  assert.equal(generated, appDelegate);
  assert.equal(configureVoiceAudio(generated), generated);
  assert.ok(
    generated.indexOf("RTCAudioSessionConfiguration.setWebRTC") <
      generated.indexOf("factory.startReactNative"),
  );
  // Inserting a default option preserves WebRTC's Bluetooth options and mode.
  assert.match(generated, /categoryOptions\.insert\(\.defaultToSpeaker\)/);
  assert.doesNotMatch(
    generated,
    /overrideOutputAudioPort|categoryOptions\s*=|outputVolume\s*=/,
  );
});

test("native generation refuses to silently omit the audio setup after a template change", () => {
  assert.throws(
    () => configureVoiceAudio("import ReactAppDependencyProvider\n"),
    /startup anchor is missing/,
  );
});
