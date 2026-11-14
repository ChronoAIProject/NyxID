import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { chainGlobalHandler, chainRejections, type RejectionOptions } from "./globalHandler";

for (const development of [false, true]) for (const hermes of [false, true]) {
  test(`${development ? "development chains RN" : "release captures only"} with ${hermes ? "Hermes" : "fallback"} rejection tracking`, () => {
    let options: RejectionOptions | undefined;
    let rnUnhandled = 0, rnHandled = 0, consoleCalls = 0, defaultsRead = 0;
    const captures: Array<[unknown, string]> = [];
    const console = {error: () => { consoleCalls++; }, warn: () => { consoleCalls++; }, log: () => { consoleCalls++; }};
    const defaults = { get default() { defaultsRead++; return {
      onUnhandled: () => { rnUnhandled++; console.error(); },
      onHandled: () => { rnHandled++; console.warn(); },
    }; } };
    const enable = (value: RejectionOptions) => { options = value; };
    const h = hookHarness<void>({
      expo: { requireOptionalNativeModule: () => ({}) },
      "expo-crypto": { randomUUID: () => "uuid" },
      "expo-constants": {expoConfig:{extra:{}}},
      "react-native": {AppState:{addEventListener(){}},Platform:{OS:"ios"}},
      "../analyticsConsentState": {analyticsConsentState:{getSnapshot(){},subscribe(){}}},
      "../api/http": {requestJson(){}},
      "./lifecycle": {createCrashLifecycle: () => ({capture: async (error:unknown,kind:string) => {captures.push([error,kind]);}})},
      "./globalHandler": {chainGlobalHandler,chainRejections},
      "react-native/Libraries/promiseRejectionTrackingOptions": defaults,
      "promise/setimmediate/rejection-tracking": {enable},
    }, {__DEV__:development,console,...(hermes ? {HermesInternal:{enablePromiseRejectionTracker:enable}} : {})});
    const runtime = h.load(fileURLToPath(new URL("./runtime.ts", import.meta.url))) as {installMobileCrashReporting():void};
    runtime.installMobileCrashReporting();
    assert.equal(options!.allRejections,true);
    const privateError = new Error("SECRET_SENTINEL rejection message");
    options!.onUnhandled!(1,privateError);
    options!.onHandled!(1);
    assert.equal(captures.length,1);
    assert.equal(captures[0]![0],privateError);
    assert.equal(captures[0]![1],"unhandled_rejection");
    assert.equal(rnUnhandled,development ? 1 : 0);
    assert.equal(rnHandled,development ? 1 : 0);
    assert.equal(defaultsRead,development ? 1 : 0);
    assert.equal(consoleCalls,development ? 2 : 0);
  });
}
