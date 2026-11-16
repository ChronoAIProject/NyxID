import assert from "node:assert/strict";
import test from "node:test";
import { setupSettingsScreen } from "./settingsTestHarness";

test("Display's device appearance description follows the app language immediately", () => {
  const env = setupSettingsScreen(
    "./DisplaySettingsScreen.tsx",
    "DisplaySettingsScreen",
  );
  try {
    env.text("Follows your device's appearance");
    env.language("zh-CN");
    env.text("跟随设备外观");
    env.language("zh-TW");
    env.text("跟隨裝置外觀");
    assert.deepEqual(env.preferences, []);
  } finally {
    env.harness.unmount();
  }
});

test("Voice settings translates the device dictation explanation with the shared language preference", () => {
  const env = setupSettingsScreen(
    "./VoiceSettingsScreen.tsx",
    "VoiceSettingsScreen",
    {
      params: { voice: false },
    },
  );
  try {
    env.text(
      "The microphone in the message field uses your device's speech recognition, with local processing when available. The words appear in your message, where you can edit them before sending.",
    );
    env.language("zh-CN");
    env.text(
      "消息输入框的麦克风使用设备的语音识别，在可用时进行本地处理。识别文字会显示在消息中，您可在发送前编辑。",
    );
    env.language("zh-TW");
    env.text(
      "訊息輸入欄的麥克風使用裝置的語音辨識，可用時會在本機處理。辨識文字會顯示在訊息中，您可在傳送前編輯。",
    );
    assert.deepEqual(env.mutations, []);
  } finally {
    env.harness.unmount();
  }
});
