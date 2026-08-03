import {
  emptyAssistantReceiptState,
  parseAssistantReceiptState,
  type AssistantCreateReceipt,
  type AssistantDeletionIntent,
  type AssistantReceiptState,
} from "@/schemas/assistant-receipts";
import { useAuthStore } from "@/stores/auth-store";

const STORAGE_PREFIX = "nyxid.assistant_receipts.v1.";
const RECEIPT_TTL_MS = 24 * 60 * 60 * 1_000;
const RECEIPT_CAP = 20;
const DELETION_INTENT_CAP = 10;

let activeOwnerUserId = useAuthStore.getState().user?.id ?? null;
const memoryByOwner = new Map<string, AssistantReceiptState>();

function storageKey(ownerUserId: string): string {
  return `${STORAGE_PREFIX}${encodeURIComponent(ownerUserId)}`;
}

function readStorage(ownerUserId: string): AssistantReceiptState | undefined {
  try {
    if (typeof localStorage === "undefined") return undefined;
    const raw = localStorage.getItem(storageKey(ownerUserId));
    if (!raw) return undefined;
    const parsed = parseAssistantReceiptState(JSON.parse(raw));
    return parsed?.ownerUserId === ownerUserId ? parsed : undefined;
  } catch {
    return undefined;
  }
}

function pruneState(
  state: AssistantReceiptState,
  now: number,
): AssistantReceiptState {
  const receipts = Object.fromEntries(
    Object.entries(state.receipts)
      .filter(([, receipt]) => now - receipt.updatedAt <= RECEIPT_TTL_MS)
      .sort(([, left], [, right]) => right.updatedAt - left.updatedAt)
      .slice(0, RECEIPT_CAP),
  );
  const deletionIntents = Object.fromEntries(
    Object.entries(state.deletionIntents)
      .filter(([, intent]) => now - intent.createdAt <= RECEIPT_TTL_MS)
      .sort(([, left], [, right]) => right.createdAt - left.createdAt)
      .slice(0, DELETION_INTENT_CAP),
  );
  return { ...state, receipts, deletionIntents };
}

function loadOwner(ownerUserId: string): AssistantReceiptState {
  const cached = memoryByOwner.get(ownerUserId);
  const state = pruneState(
    cached ??
      readStorage(ownerUserId) ??
      emptyAssistantReceiptState(ownerUserId),
    Date.now(),
  );
  memoryByOwner.set(ownerUserId, state);
  return state;
}

function persist(state: AssistantReceiptState): void {
  const ownerUserId = state.ownerUserId;
  if (!ownerUserId) return;
  const pruned = pruneState(state, Date.now());
  memoryByOwner.set(ownerUserId, pruned);
  try {
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(storageKey(ownerUserId), JSON.stringify(pruned));
    }
  } catch {
    // The in-memory namespace remains authoritative for this tab.
  }
}

function activeState(): AssistantReceiptState | undefined {
  const ownerUserId = activeOwnerUserId;
  if (!ownerUserId) return undefined;
  return loadOwner(ownerUserId);
}

function updateActive(
  update: (state: AssistantReceiptState) => AssistantReceiptState,
): void {
  const current = activeState();
  if (!current || current.ownerUserId !== activeOwnerUserId) return;
  persist(update(current));
}

function deleteReceiptForOwner(ownerUserId: string, commandId: string): void {
  const state = loadOwner(ownerUserId);
  if (!(commandId in state.receipts)) return;
  const receipts = { ...state.receipts };
  delete receipts[commandId];
  persist({ ...state, receipts });
}

export function recordCreateReceipt(
  commandId: string,
  placeholderId: string,
  now: number = Date.now(),
): void {
  updateActive((state) => ({
    ...state,
    receipts: {
      ...state.receipts,
      [commandId]: {
        placeholderId,
        createdAt: state.receipts[commandId]?.createdAt ?? now,
        updatedAt: now,
      },
    },
  }));
}

export function adoptReceiptIdentity(
  commandId: string,
  conversationId: string,
  stateVersion?: number,
  now: number = Date.now(),
): void {
  updateActive((state) => {
    const receipt = state.receipts[commandId];
    if (!receipt) return state;
    const usableFence =
      Number.isSafeInteger(stateVersion) && (stateVersion ?? 0) > 0
        ? stateVersion
        : undefined;
    return {
      ...state,
      receipts: {
        ...state.receipts,
        [commandId]: {
          ...receipt,
          conversationId,
          stateVersion:
            usableFence === undefined
              ? receipt.stateVersion
              : Math.max(receipt.stateVersion ?? 0, usableFence),
          updatedAt: now,
        },
      },
    };
  });
}

export function advanceReceiptFence(
  conversationId: string,
  stateVersion: number,
  now: number = Date.now(),
): void {
  if (!Number.isSafeInteger(stateVersion) || stateVersion <= 0) return;
  updateActive((state) => {
    const match = Object.entries(state.receipts).find(
      ([, receipt]) => receipt.conversationId === conversationId,
    );
    if (!match) return state;
    const [commandId, receipt] = match;
    return {
      ...state,
      receipts: {
        ...state.receipts,
        [commandId]: {
          ...receipt,
          stateVersion: Math.max(receipt.stateVersion ?? 0, stateVersion),
          updatedAt: now,
        },
      },
    };
  });
}

export function deleteReceipt(commandId: string): void {
  updateActive((state) => {
    if (!(commandId in state.receipts)) return state;
    const receipts = { ...state.receipts };
    delete receipts[commandId];
    return { ...state, receipts };
  });
}

export function retireReceiptAfterMaterialization(
  commandId: string,
  now: number = Date.now(),
): void {
  const ownerUserId = activeOwnerUserId;
  const receipt = activeState()?.receipts[commandId];
  if (!ownerUserId || !receipt) return;
  const delayMs = Math.max(0, receipt.createdAt + 60_000 - now);
  setTimeout(() => deleteReceiptForOwner(ownerUserId, commandId), delayMs);
}

export function findReceiptByPlaceholder(
  placeholderId: string,
): (AssistantCreateReceipt & { readonly commandId: string }) | undefined {
  const match = Object.entries(activeState()?.receipts ?? {}).find(
    ([, receipt]) => receipt.placeholderId === placeholderId,
  );
  return match ? { commandId: match[0], ...match[1] } : undefined;
}

export function findReceiptByConversation(
  conversationId: string,
): (AssistantCreateReceipt & { readonly commandId: string }) | undefined {
  const match = Object.entries(activeState()?.receipts ?? {}).find(
    ([, receipt]) => receipt.conversationId === conversationId,
  );
  return match ? { commandId: match[0], ...match[1] } : undefined;
}

export function recordDeletionIntent(
  commandId: string,
  placeholderId: string,
  conversationId?: string,
  now: number = Date.now(),
): void {
  updateActive((state) => ({
    ...state,
    deletionIntents: {
      ...state.deletionIntents,
      [commandId]: {
        placeholderId,
        conversationId:
          conversationId ?? state.deletionIntents[commandId]?.conversationId,
        createdAt: state.deletionIntents[commandId]?.createdAt ?? now,
      },
    },
  }));
}

export function resolveDeletionIntent(commandId: string): void {
  updateActive((state) => {
    if (!(commandId in state.deletionIntents)) return state;
    const deletionIntents = { ...state.deletionIntents };
    delete deletionIntents[commandId];
    return { ...state, deletionIntents };
  });
}

export function listDeletionIntents(): readonly (AssistantDeletionIntent & {
  readonly commandId: string;
})[] {
  return Object.entries(activeState()?.deletionIntents ?? {}).map(
    ([commandId, intent]) => ({ commandId, ...intent }),
  );
}

useAuthStore.subscribe((state, previousState) => {
  const nextOwnerUserId = state.user?.id ?? null;
  const previousOwnerUserId = previousState.user?.id ?? null;
  if (nextOwnerUserId === previousOwnerUserId) return;
  activeOwnerUserId = nextOwnerUserId;
  if (nextOwnerUserId) loadOwner(nextOwnerUserId);
});

if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    const ownerUserId = activeOwnerUserId;
    if (!ownerUserId || event.key !== storageKey(ownerUserId)) return;
    memoryByOwner.delete(ownerUserId);
    loadOwner(ownerUserId);
  });
}

export function resetAssistantReceiptStoreForTests(): void {
  memoryByOwner.clear();
  activeOwnerUserId = useAuthStore.getState().user?.id ?? null;
}
