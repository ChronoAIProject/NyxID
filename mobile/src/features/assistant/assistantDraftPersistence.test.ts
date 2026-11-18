import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";
import { createSessionCoordinator } from "../../lib/auth/sessionCoordinator";

test("a fresh JS process restores only the current account's scoped, encrypted text", async () => {
  const secure = new Map<string, string>();
  const disk = new Map<string, string>();
  const keychain = {
    getItemAsync: async (key: string) => secure.get(key) ?? null,
    setItemAsync: async (key: string, value: string) => { secure.set(key, value); },
    deleteItemAsync: async (key: string) => { secure.delete(key); },
  };
  let account: string | null = "owner";
  let epoch = 0;
  function launch() {
    const coordinator = createSessionCoordinator(keychain, () => {});
    let accountChanged = () => {};
    const h = hookHarness({
      expo: { NativeModule: class {}, registerWebModule: (Module: new () => object) => new Module() },
      "@react-native-async-storage/async-storage": {
        getAllKeys: async () => [...disk.keys()],
        multiRemove: async (keys: string[]) => { for (const key of keys) disk.delete(key); },
        getItem: async (key: string) => disk.get(key),
        setItem: async (key: string, value: string) => { disk.set(key, value); },
        removeItem: async (key: string) => { disk.delete(key); },
      },
      "../../lib/auth/authAccount": { getAuthAccountId: () => account, subscribeAuthAccount: (callback: () => void) => { accountChanged = callback; } },
      "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
      "../../lib/auth/sessionStore": { authSessionGeneration: coordinator.generation, sessionDraftKey: coordinator.draftKey },
    }, { TextEncoder, TextDecoder, crypto: globalThis.crypto, Uint8Array, ArrayBuffer, atob, btoa });
    const aes = join(dirname(createRequire(import.meta.url).resolve("expo-crypto/package.json")), "src/aes");
    h.mock("./web-utils", h.load(join(aes, "web-utils.ts")));
    h.mock("./aes.types", h.load(join(aes, "aes.types.ts")));
    h.mock("./ExpoCryptoAES", h.load(join(aes, "ExpoCryptoAES.web.ts")));
    h.mock("expo-crypto", h.load(join(aes, "index.ts")));
    return { accountChanged: () => accountChanged(), ...h.load(fileURLToPath(new URL("./assistantDraftPersistence.ts", import.meta.url))) as typeof import("./assistantDraftPersistence"), coordinator };
  }
  const original = launch();
  const text = "我的草稿🎙".repeat(6000);
  await Promise.all([
    original.saveAssistantDraft("home", "initial", epoch),
    original.saveAssistantDraft("home", text, epoch),
    original.saveAssistantDraft("chat-one", "private reply", epoch),
  ]);
  assert.equal(disk.size, 2);
  assert.ok([...disk.values()].every((v) => !v.includes("草稿") && !v.includes("private reply")));
  const reopened = launch();
  assert.equal(await reopened.readAssistantDraft("home", epoch), text);
  assert.equal(await reopened.readAssistantDraft("chat-one", epoch), "private reply");
  assert.equal(await reopened.readAssistantDraft("other-chat", epoch), undefined);
  const stale = reopened.readAssistantDraft("home", epoch);
  epoch++;
  account = "other-owner";
  reopened.accountChanged();
  assert.equal(await stale, undefined);
  assert.equal(await reopened.readAssistantDraft("home", epoch), undefined);
  assert.equal(disk.size, 0);
  await reopened.coordinator.clear();
  account = "owner";
  epoch++;
  assert.equal(await launch().readAssistantDraft("home", epoch), undefined);
  await reopened.saveAssistantDraft("chat-one", "", epoch);
  assert.equal(await reopened.readAssistantDraft("chat-one", epoch), undefined);
});
