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
