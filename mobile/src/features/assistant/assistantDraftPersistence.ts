import AsyncStorage from "@react-native-async-storage/async-storage";
import { AESEncryptionKey, AESSealedData, aesDecryptAsync, aesEncryptAsync } from "expo-crypto";
import { getAuthAccountId, subscribeAuthAccount } from "../../lib/auth/authAccount";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { authSessionGeneration, sessionDraftKey } from "../../lib/auth/sessionStore";

const prefix = "nyxid.assistant.draft.v1:";
const encoder = new TextEncoder();
let pending: Promise<unknown> = Promise.resolve();
const latestWrite = new Map<string, symbol>();

function serialize<T>(operation: () => Promise<T>) {
  const result = pending.then(operation);
  pending = result.catch(() => undefined);
  return result;
}

// Dispose the prior account's encrypted rows in write order. Login and logout
// also discard its key together with the credentials, including after a crash.
let previousAccount = getAuthAccountId();
subscribeAuthAccount(() => {
  const owner = previousAccount;
  previousAccount = getAuthAccountId();
  if (!owner) return;
  const accountPrefix = prefix + JSON.stringify([owner]).slice(0, -1) + ",";
  void serialize(async () => {
    const rows = (await AsyncStorage.getAllKeys()).filter((key) => key.startsWith(accountPrefix));
    if (rows.length) await AsyncStorage.multiRemove(rows);
  }).catch(() => undefined);
});

function scope(draft: string, epoch: number) {
  const owner = getAuthAccountId();
  if (!owner || epoch !== getAuthEpoch()) return;
  const generation = authSessionGeneration();
  return {
    row: prefix + JSON.stringify([owner, draft]),
    generation,
    current: () => owner === getAuthAccountId() && epoch === getAuthEpoch() && generation === authSessionGeneration(),
  };
}

export function readAssistantDraft(draft: string, epoch: number): Promise<string | undefined> {
  const target = scope(draft, epoch);
  if (!target) return Promise.resolve(undefined);
  return serialize(async () => {
    if (!target.current()) return;
    const ciphertext = await AsyncStorage.getItem(target.row);
    if (!ciphertext) return;
    const key = await sessionDraftKey(target.generation);
    if (!key || !target.current()) return;
    const plaintext = await aesDecryptAsync(
      AESSealedData.fromCombined(ciphertext),
      await AESEncryptionKey.import(key, "base64"),
      { additionalData: encoder.encode(target.row) },
    );
    return target.current() ? new TextDecoder().decode(plaintext) : undefined;
  });
}

export function saveAssistantDraft(draft: string, text: string, epoch: number): Promise<void> {
  const target = scope(draft, epoch);
  if (!target) return Promise.resolve();
  const write = Symbol();
  latestWrite.set(target.row, write);
  const current = () => target.current() && latestWrite.get(target.row) === write;
  return serialize(async () => {
    if (!current()) return;
    if (!text) {
      await AsyncStorage.removeItem(target.row);
      return;
    }
    const raw = await sessionDraftKey(target.generation, async () => (await AESEncryptionKey.generate()).encoded("base64"));
    if (!raw || !current()) return;
    const sealed = await aesEncryptAsync(encoder.encode(text), await AESEncryptionKey.import(raw, "base64"), {
      additionalData: encoder.encode(target.row),
    });
    const ciphertext = await sealed.combined("base64");
    if (current()) await AsyncStorage.setItem(target.row, ciphertext);
  }).finally(() => {
    if (latestWrite.get(target.row) === write) latestWrite.delete(target.row);
  });
}
