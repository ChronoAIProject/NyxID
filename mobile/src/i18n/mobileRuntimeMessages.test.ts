import assert from "node:assert/strict";
import test from "node:test";
import { translate } from "./translate";
import simplified from "./locales/zh-CN.json";
import traditional from "./locales/zh-TW.json";

// These permission and offline states must never fall back to English in a Chinese UI.
const runtimeCopy = [
  "Added skills unavailable offline",
  "Allow microphone access in Settings to dictate. You can keep typing.",
  "Allow microphone and speech recognition access in Settings to dictate. You can keep typing.",
  "Dictation could not start. Please try again. You can keep typing.",
  "Dictation could not start. You can keep typing.",
  "Dictation isn't available in your device's language. You can keep typing.",
  "Dictation isn't available in your selected language. Choose another language or keep typing.",
  "Dictation stopped unexpectedly. You can keep typing.",
  "Dictation was interrupted.",
  "Keep NyxID open to start a voice call.",
  "Microphone access is off. Allow it in your device settings to use voice; you can keep chatting with text.",
  "Microphone permission took too long. Try voice again and respond to the permission prompt.",
  "No speech was detected. Try again.",
  "Open your device Settings to allow microphone access.",
  "Open your device Settings to allow microphone and speech recognition access.",
  "Reconnect before changing this agent's skills.",
  "Reconnect to load agents and their skills.",
  "Reconnect to load available connectors.",
  "Reconnect to load published skill versions.",
  "Reconnect to load skill history.",
  "Reconnect to load skill information.",
  "Reconnect to load these connections.",
  "Reconnect to load this agent's skills.",
  "Reconnect to load this package preview.",
  "Reconnect to load this skill's agent.",
  "Reconnect to load your added connectors.",
  "Reconnect to load your plugins.",
  "Reconnect to search the skill catalog.",
  "Showing cached plugins. Reconnect to refresh.",
  "Showing cached skill information. Reconnect to refresh or make changes.",
  "Skill catalog unavailable offline",
  "Speech recognition is busy. Try again in a moment.",
  "Speech recognition is restricted on this device in Screen Time. You can keep typing.",
  "Speech recognition is restricted on this device. You can keep typing.",
  "Speech recognition isn't available right now. Check microphone access and your device's speech recognition service, then try again. You can keep typing.",
  "Speech recognition isn't available right now. Check that Dictation is on in Settings › General › Keyboard, then try again.",
  "Speech recognition needs a connection for your language. Check your connection and try again.",
  "Speech wasn't finalized. Please try again.",
  "The microphone couldn't record. Close other apps using it and try again.",
  "Voice could not open. Try again.",
  "Voice could not start. Try again.",
  "Voice options could not load.",
  "This device doesn't have a speech recognition service available. You can keep typing.",
  "Your device's language",
] as const;

test("Android runtime messages are registered in both shared app catalogs", () => {
  for (const phrase of runtimeCopy) {
    assert.equal(translate("en", phrase), phrase);
    for (const [language, catalog] of [
      ["zh-CN", simplified],
      ["zh-TW", traditional],
    ] as const) {
      assert.ok(Object.hasOwn(catalog, phrase), `${language}: ${phrase}`);
      const translated = translate(language, phrase);
      assert.ok(translated.trim());
      assert.notEqual(translated, phrase, `${language}: ${phrase}`);
    }
  }
});
