import type { AssistantUploadSession } from "./AssistantUploadSession";
import { getAuthAccountId } from "../../lib/auth/authAccount";
import { getAuthEpoch } from "../../lib/auth/authEpoch";

/** Session-only drafts are scoped to an account and never written to disk. */
export function createAssistantDraftStore(epoch: () => number) {
  let owner = epoch();
  const rows = new Map<string, { value: unknown; dispose?: () => void }>();
  function sync() {
    if (owner === epoch()) return;
    const stale = [...rows.values()];
    rows.clear();
    owner = epoch();
    stale.forEach((row) => row.dispose?.());
  }
  return {
    get<T>(key: string): T | undefined {
      sync();
      return rows.get(key)?.value as T | undefined;
    },
    set<T>(key: string, value: T, dispose?: () => void) {
      sync();
      rows.set(key, { value, dispose });
    },
    delete(key: string) {
      sync();
      rows.delete(key);
    },
    clear() {
      const stale = [...rows.values()];
      rows.clear();
      stale.forEach((row) => row.dispose?.());
    },
  };
}
export const assistantDraftStore = createAssistantDraftStore(getAuthEpoch);
export function assistantDraftKey(
  agentId?: string,
  conversationId?: string,
  draftId?: string,
  home = false,
) {
  if (conversationId) return JSON.stringify(["private", conversationId]);
  if (home) return "nyxbot-home";
  return JSON.stringify(["private", agentId ?? "nyxbot", draftId ?? "new"]);
}

/** Release an explicitly discarded draft or the attachments of a removed chat. */
const discards = new Map<string, Promise<void>>();
export function pendingUploadDiscard(owner: string, key: string) {
  return discards.get(JSON.stringify([owner, key]));
}

export function discardUploadDraft(owner: string | null, key: string) {
  if (!owner || getAuthAccountId() !== owner) return Promise.resolve();
  const epoch = getAuthEpoch();
  const operationKey = JSON.stringify([owner, key]);
  const pending = discards.get(operationKey);
  if (pending) return pending;

  const operation = (async () => {
    const session = assistantDraftStore.get<AssistantUploadSession>(`uploads:${key}`);
    await session?.prepareDiscard();
    const { discardStoredUploadDraft, uploadDraftAuthority } = await import("./uploadDraftStorage");
    const guard = uploadDraftAuthority(owner, epoch);
    guard();
    await discardStoredUploadDraft(owner, key, epoch);
    guard();
    assistantDraftStore.get<AssistantUploadSession>(`uploads:${key}`)?.suspend();
    assistantDraftStore.delete(`uploads:${key}`);
    assistantDraftStore.delete(`text:${key}`);
  })().finally(() => discards.delete(operationKey));
  discards.set(operationKey, operation);
  return operation;
}
