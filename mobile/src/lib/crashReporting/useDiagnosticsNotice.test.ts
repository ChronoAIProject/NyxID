import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { translate } from "../../i18n/translate";
import type { Language } from "../../i18n/language";
const tick = () => new Promise((resolve) => setImmediate(resolve));
for (const language of ["en", "zh-CN", "zh-TW"] as Language[])
  test(`updated notice is account-bound, localized and never changes permission (${language})`, async () => {
    const stored = new Map<string, string>();
    const alerts: Array<{
      title: string;
      message: string;
      buttons: Array<{ text: string; onPress?: () => void }>;
    }> = [];
    let reviews = 0;
    const consent = {
      accountId: "a",
      isHydrated: true,
      enabled: true,
      policy: 1,
    };
    let permission = {
      account_id: "a",
      enabled: true,
      policy_version: "1",
      current_policy_version: "2",
      diagnostics_offered: true,
    };
    const t = (phrase: string) => translate(language, phrase);
    const h = hookHarness<void>({
      "react-native": {
        Alert: {
          alert: (
            title: string,
            message: string,
            buttons: Array<{ text: string; onPress?: () => void }>,
          ) => alerts.push({ title, message, buttons }),
        },
      },
      "@react-native-async-storage/async-storage": {
        getItem: async (k: string) => stored.get(k) ?? null,
        setItem: async (k: string, v: string) => {
          stored.set(k, v);
        },
      },
      "../publicConfigHints": { getCrashDiagnosticsOffered: () => true, subscribePublicConfigHints: () => () => {} },
      "../consent": { useMobileConsent: () => consent },
      "../api/http": { requestJson: async () => permission },
      "../../i18n/LanguageProvider": { useAppLanguage: () => ({ t }) },
    });
    const { useDiagnosticsNotice } = h.load(
      fileURLToPath(new URL("./useDiagnosticsNotice.ts", import.meta.url)),
    ) as {
      useDiagnosticsNotice: (loggedIn: boolean, review: () => void) => void;
    };
    const review = () => {
      reviews++;
    };
    h.mount(() => useDiagnosticsNotice(true, review));
    await tick();
    assert.equal(alerts.length, 1);
    assert.equal(alerts[0]!.title, t("Analytics updated"));
    assert.equal(alerts[0]!.buttons[0]!.text, t("Not now"));
    assert.equal(consent.policy, 1);
    assert.equal(reviews, 0);
    alerts[0]!.buttons[1]!.onPress!();
    assert.equal(reviews, 1);
    h.render();
    await tick();
    assert.equal(alerts.length, 1);
    consent.accountId = "b";
    permission = { ...permission, account_id: "b" };
    h.render();
    await tick();
    assert.equal(alerts.length, 2);
    h.unmount();
  });

test("disabled diagnostics never displays an updated analytics alert", async () => {
  let alerts = 0;
  const h = hookHarness<void>({
    "react-native": { Alert: { alert: () => { alerts++; } } },
    "@react-native-async-storage/async-storage": { getItem: async () => null, setItem: async () => assert.fail("no dismissal while not offered") },
    "../publicConfigHints": { getCrashDiagnosticsOffered: () => true, subscribePublicConfigHints: () => () => {} },
    "../consent": { useMobileConsent: () => ({accountId:"a",isHydrated:true,enabled:true,policy:1}) },
    "../api/http": { requestJson: async () => ({account_id:"a",enabled:true,policy_version:"1",current_policy_version:"1",diagnostics_offered:false}) },
    "../../i18n/LanguageProvider": { useAppLanguage: () => ({t:(s:string)=>s}) },
  });
  const {useDiagnosticsNotice} = h.load(fileURLToPath(new URL("./useDiagnosticsNotice.ts", import.meta.url))) as {useDiagnosticsNotice:(authenticated:boolean,review:()=>void)=>void};
  h.mount(() => useDiagnosticsNotice(true, () => assert.fail("not offered")));
  await tick();
  assert.equal(alerts,0);
  h.unmount();
});

for (const hint of [false, true]) test(`public crash hint ${hint ? "present permits" : "absent prevents"} notice preference reads`, async () => {
  let requests = 0, alerts = 0, enabled = hint;
  let changed: (() => void) | undefined;
  const h = hookHarness<void>({
    "react-native": { Alert: { alert: () => { alerts++; } } },
    "@react-native-async-storage/async-storage": { getItem: async () => null, setItem: async () => {} },
    "../publicConfigHints": {
      getCrashDiagnosticsOffered: () => enabled,
      subscribePublicConfigHints: (listener: () => void) => { changed = listener; return () => {}; },
    },
    "../consent": { useMobileConsent: () => ({ accountId: "a", isHydrated: true, enabled: true, policy: 1 }) },
    "../api/http": { requestJson: async () => {
      requests++;
      return { account_id: "a", enabled: true, policy_version: "1", current_policy_version: "2", diagnostics_offered: true };
    } },
    "../../i18n/LanguageProvider": { useAppLanguage: () => ({ t: (s: string) => s }) },
  });
  const { useDiagnosticsNotice } = h.load(fileURLToPath(new URL("./useDiagnosticsNotice.ts", import.meta.url))) as { useDiagnosticsNotice: (a: boolean, r: () => void) => void };
  const review = () => {};
  h.mount(() => useDiagnosticsNotice(true, review));
  await tick();
  assert.equal(requests, hint ? 1 : 0);
  assert.equal(alerts, hint ? 1 : 0);
  if (!hint) {
    h.render(); await tick();
    assert.equal(requests, 0);
    enabled = true; changed!(); h.flush(); await tick();
    assert.equal(requests, 1);
    assert.equal(alerts, 1);
  }
  h.unmount();
});
