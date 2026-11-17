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
      "This language is used for message dictation and crew voice dictation. Voice conversations use their own language preference. Words dictated into a message can be edited before sending.",
    );
    env.language("zh-CN");
    env.text(
      "此语言用于消息听写和团队语音听写。语音通话使用独立的语言偏好。听写到消息中的文字可以在发送前编辑。",
    );
    env.language("zh-TW");
    env.text(
      "此語言用於訊息聽寫和團隊語音聽寫。語音通話使用獨立的語言偏好。聽寫到訊息中的文字可以在傳送前編輯。",
    );
    assert.deepEqual(env.mutations, []);
  } finally {
    env.harness.unmount();
  }
});
