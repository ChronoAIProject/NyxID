import assert from "node:assert/strict";
import test from "node:test";
import {
  recognitionLocales,
  LOCALE_DISCOVERY_TIMEOUT_MS,
  manualDictationLocale,
} from "./recognitionLocales";

test("Android queries the default provider that will receive the recording, even without on-device recognition", async () => {
  const calls: unknown[] = [];
  const result = await recognitionLocales(
    {
      getDefaultRecognitionService: () => ({ packageName: "qa.recognizer" }),
      getSupportedLocales: async (options) => {
        calls.push(options);
        return { locales: ["zh-CN", "en-US", "zh-CN"], installedLocales: [] };
      },
    },
    "android",
  );
  assert.deepEqual(calls, [
    { androidRecognitionServicePackage: "qa.recognizer" },
  ]);
  assert.deepEqual(result, {
    locales: ["zh-CN", "en-US"],
    installedLocales: [],
    service: "qa.recognizer",
  });
});
for (const mode of ["empty", "reject", "never"] as const) {
  test(`a ${mode} provider returns a bounded unavailable result without inventing support`, async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const pending = recognitionLocales(
      {
        getSupportedLocales: async () => {
          if (mode === "reject") throw new Error("Unsupported discovery");
          if (mode === "never") return new Promise(() => {});
          return { locales: [], installedLocales: [] };
        },
      },
      "android",
    );
    t.mock.timers.tick(LOCALE_DISCOVERY_TIMEOUT_MS);
    assert.deepEqual((await pending).locales, []);
  });
}
test("iOS retains its native on-device discovery and installed locales", async () => {
  const result = await recognitionLocales(
    {
      getDefaultRecognitionService: () => {
        throw new Error("Android only");
      },
      getSupportedLocales: async (options) => {
        assert.deepEqual(options, {});
        return { locales: ["ja-JP"], installedLocales: ["ja-JP"] };
      },
    },
    "ios",
  );
  assert.deepEqual(result.installedLocales, ["ja-JP"]);
});
test("manual language tags normalize valid codes and reject search text or arbitrary extras", () => {
  for (const [input, expected] of [
    [" ZH_cn ", "zh-CN"],
    ["zh-hant-tw", "zh-Hant-TW"],
    ["ja", "ja"],
    ["en-001", "en-001"],
  ])
    assert.equal(manualDictationLocale(input!), expected);
  for (const input of [
    "",
    "English",
    "zh CN",
    "xx-/oops",
    "en-US;lang=ja-JP",
    "en-u-ca-gregory",
  ])
    assert.equal(manualDictationLocale(input), undefined);
});
