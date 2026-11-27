import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { wrongScriptCharacters } from "../../i18n/i18nScriptGuard";
import { onboardingText } from "../../../../shared/onboardingCopy";
import { EMPTY, replayScript } from "../../../../shared/onboardingReplay";
import simplified from "../../../../shared/i18n/onboarding/zh-CN.json";
import traditional from "../../../../shared/i18n/onboarding/zh-TW.json";

test("both Chinese scripts cover every server-owned onboarding recommendation, follow-up and topic", () => {
  const service = readFileSync(new URL("../../../../backend/src/services/assistant_onboarding_service.rs", import.meta.url), "utf8");
  const connect = readFileSync(new URL("../../../../backend/src/services/assistant_onboarding_connect.rs", import.meta.url), "utf8");
  const recommendations = Array.from(service.matchAll(/^\s*(?:title|description|prompt): "(.+)",$/gm), m => m[1]!);
  const choices = Array.from(service.matchAll(/=> \("([^"]+)", "[^"]+"\)/g), m => m[1]!);
  const followups = Array.from(connect.split('prompt: format!(')[1]!.matchAll(/=>\s*"([^"]+)",/g), m => "Use my existing connected account. " + m[1]);
  assert.ok(recommendations.length && choices.length && followups.length);
  assert.deepEqual(Object.keys(simplified).sort(), Object.keys(traditional).sort());
  for (const key of [...Object.keys(simplified), ...choices, ...recommendations, ...followups]) {
    assert.equal(onboardingText("en", key), key);
    for (const language of ["zh-CN", "zh-TW"] as const) {
      const phrase = onboardingText(language, key);
      assert.match(phrase, /\p{Script=Han}/u, key);
      assert.deepEqual(wrongScriptCharacters(language, phrase), [], key);
    }
  }
  assert.equal(onboardingText("zh-CN", "My own unrecognized text"), "My own unrecognized text");
});

test("all replay variants localize typed text, responses, status and approval copy before animation starts", () => {
  for (const language of ["zh-CN", "zh-TW"]) {
    for (const kind of ["chief-of-staff", "specialists", "control"] as const) {
      for (const topic of ["inbox", "calendar", "customers", "sales", "coding", "research", "unknown"]) {
        let scene = EMPTY;
        for (const beat of replayScript(kind, [topic], language)) {
          scene = beat.apply(scene);
          if (scene.typed) assert.match(scene.typed, /\p{Script=Han}/u);
          if (scene.working) assert.match(scene.working, /\p{Script=Han}/u);
          if (scene.card) assert.match(scene.card.row.summary, /\p{Script=Han}.*ci-bot/u);
          for (const message of scene.messages) {
            if (message.content) assert.match(message.content, /\p{Script=Han}/u);
          }
        }
      }
    }
  }
});
