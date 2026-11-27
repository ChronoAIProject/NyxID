import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { translate } from "../../i18n/translate";
import type { Language } from "../../i18n/language";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import { elements, type Element } from "./settingsTestHarness";

function setup({ configured = true, language = "en" as Language } = {}) {
  let epoch: number | null = null;
  let nextEpoch = 1;
  let unsubscribed = 0;
  const listeners = new Set<() => void>();
  const loads: ReturnType<typeof deferred<void>>[] = [];
  const saves: {
    enabled: boolean;
    result: ReturnType<typeof deferred<void>>;
  }[] = [];
  const publish = (value: number | null) => {
    epoch = value;
    for (const listener of listeners) listener();
  };
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const harness = hookHarness<Element | null>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": { Switch: "Switch" },
    "lucide-react-native": { ChartColumn: "ChartColumn" },
    "../../i18n/LanguageProvider": {
      useAppLanguage: () => ({
        t: (phrase: string) => translate(language, phrase),
      }),
    },
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsRow: "SettingsRow",
    },
    "../../lib/performance/metrics": {
      isPerformanceConfigured: () => configured,
      getPerformanceConsentEpoch: () => epoch,
      subscribePerformanceConsent: (listener: () => void) => {
        listeners.add(listener);
        return () => {
          unsubscribed++;
          listeners.delete(listener);
        };
      },
      hydratePerformanceConsent: () => {
        const result = deferred<void>();
        loads.push(result);
        return result.promise;
      },
      setPerformanceConsent: (enabled: boolean) => {
        if (!enabled) publish(null);
        const result = deferred<void>();
        saves.push({ enabled, result });
        return result.promise.then(() => {
          if (enabled) publish(nextEpoch++);
        });
      },
    },
  });
  const { PerformanceConsentSettings } = harness.load(
    fileURLToPath(new URL("./PerformanceConsentSettings.tsx", import.meta.url)),
  ) as { PerformanceConsentSettings: () => Element | null };
  harness.mount(PerformanceConsentSettings);
  const all = () => (harness.value() ? elements(harness.value()!) : []);
  const row = (phrase: string) =>
    all().find((node) => node.props.label === translate(language, phrase))!;
  const control = () =>
    row("Share performance metrics").props.accessory as Element;
  const flush = async () => {
    for (let step = 0; step < 5; step++) {
      await Promise.resolve();
      harness.flush();
    }
  };
  return {
    harness,
    all,
    row,
    control,
    loads,
    saves,
    publish,
    flush,
    listeners,
    unsubscribed: () => unsubscribed,
    async ready(value: number | null = null) {
      publish(value);
      loads.at(-1)!.resolve();
      await flush();
    },
  };
}

test("unconfigured performance collection has no privacy control or storage reads", () => {
  const env = setup({ configured: false });
  assert.equal(env.harness.value(), null);
  assert.equal(env.loads.length, 0);
  assert.equal(env.listeners.size, 0);
  env.harness.unmount();
});

test("configured consent starts off and disabled until its local choice loads", async () => {
  const env = setup();
  assert.equal(env.control().props.value, false);
  assert.equal(env.control().props.disabled, true);
  assert.equal(env.control().props.accessibilityState.busy, true);
  env.control().props.onValueChange(true);
  assert.deepEqual(env.saves, []);
  await env.ready();
  assert.equal(env.control().props.value, false);
  assert.equal(env.control().props.disabled, false);
  assert.equal(env.control().props.accessibilityState.busy, false);
  assert.match(
    env.harness.value()!.props.footer,
    /independent of account usage analytics/,
  );
  env.harness.unmount();
});

test("a saved local choice and later same-device consent changes update the switch", async () => {
  const env = setup();
  await env.ready(42);
  assert.equal(env.control().props.value, true);
  env.publish(null);
  env.harness.flush();
  assert.equal(env.control().props.value, false);
  env.publish(43);
  env.harness.flush();
  assert.equal(env.control().props.value, true);
  env.harness.unmount();
  assert.equal(env.listeners.size, 0);
  assert.equal(env.unsubscribed(), 1);
});

test("enabling waits for persistence and blocks duplicate save events", async () => {
  const env = setup();
  await env.ready();
  env.control().props.onValueChange(true);
  env.control().props.onValueChange(true);
  env.harness.flush();
  assert.equal(env.saves.length, 1);
  assert.equal(env.saves[0]!.enabled, true);
  assert.equal(env.control().props.value, false);
  assert.equal(env.control().props.disabled, true);
  env.saves[0]!.result.resolve();
  await env.flush();
  assert.equal(env.control().props.value, true);
  assert.equal(env.control().props.disabled, false);
  env.harness.unmount();
});

test("a failed disable stays off in this session and retries the disabled choice", async () => {
  const env = setup();
  await env.ready(2);
  env.control().props.onValueChange(false);
  env.harness.flush();
  assert.equal(
    env.control().props.value,
    false,
    "withdrawal takes effect before storage settles",
  );
  env.saves[0]!.result.reject(new Error("storage unavailable"));
  await env.flush();
  assert.equal(env.control().props.value, false);
  assert.equal(env.control().props.disabled, false);
  const retry = env.row("Retry saving performance choice");
  assert.match(retry.props.detail, /off for this session/);
  assert.match(retry.props.detail, /before closing the app/);
  retry.props.onPress();
  assert.deepEqual(
    env.saves.map((save) => save.enabled),
    [false, false],
  );
  env.saves[1]!.result.resolve();
  await env.flush();
  assert.equal(env.control().props.value, false);
  assert.equal(env.row("Retry saving performance choice"), undefined);
  env.harness.unmount();
});

test("a failed enable remains off and retries only the explicit enable request", async () => {
  const env = setup();
  await env.ready();
  env.control().props.onValueChange(true);
  env.saves[0]!.result.reject(new Error("storage unavailable"));
  await env.flush();
  assert.equal(env.control().props.value, false);
  assert.match(
    env.row("Retry saving performance choice").props.detail,
    /remains off/,
  );
  env.row("Retry saving performance choice").props.onPress();
  assert.deepEqual(
    env.saves.map((save) => save.enabled),
    [true, true],
  );
  env.saves[1]!.result.resolve();
  await env.flush();
  assert.equal(env.control().props.value, true);
  assert.equal(env.row("Retry saving performance choice"), undefined);
  env.harness.unmount();
});

test("a later consent enable on this device clears stale off-session feedback", async () => {
  const env = setup();
  await env.ready(2);
  env.control().props.onValueChange(false);
  env.saves[0]!.result.reject(new Error("storage unavailable"));
  await env.flush();
  assert.ok(env.row("Retry saving performance choice"));
  env.publish(3);
  env.harness.flush();
  assert.equal(env.control().props.value, true);
  assert.equal(env.row("Retry saving performance choice"), undefined);
  env.harness.unmount();
});

test("a failed load stays off and offers a separate storage-read retry", async () => {
  const env = setup();
  env.loads[0]!.reject(new Error("storage unavailable"));
  await env.flush();
  assert.equal(env.control().props.value, false);
  assert.equal(env.control().props.disabled, true);
  assert.equal(env.control().props.accessibilityState.busy, false);
  assert.match(
    env.row("Retry loading performance choice").props.detail,
    /could not be loaded/,
  );
  env.row("Retry loading performance choice").props.onPress();
  env.harness.flush();
  assert.equal(env.loads.length, 2);
  assert.equal(env.control().props.accessibilityState.busy, true);
  await env.ready(8);
  assert.equal(env.control().props.value, true);
  assert.equal(env.control().props.disabled, false);
  assert.equal(env.row("Retry loading performance choice"), undefined);
  assert.equal(env.saves.length, 0);
  env.harness.unmount();
});

for (const language of ["en", "zh-CN", "zh-TW"] as const) {
  test(`performance choice and persistence recovery use ${language} copy`, async () => {
    const env = setup({ language });
    await env.ready(10);
    assert.equal(
      env.control().props.accessibilityLabel,
      translate(language, "Share performance metrics on this device"),
    );
    env.control().props.onValueChange(false);
    env.saves[0]!.result.reject(new Error("write failed"));
    await env.flush();
    const error = env.row("Retry saving performance choice");
    assert.ok(error);
    if (language !== "en") {
      assert.notEqual(env.harness.value()!.props.title, "Performance metrics");
      assert.notEqual(error.props.label, "Retry saving performance choice");
      assert.doesNotMatch(error.props.detail, /off for this session/);
    }
    env.harness.unmount();
  });
}
