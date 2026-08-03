import { beforeEach, describe, expect, it, vi } from "vitest";
import type { User } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import {
  adoptReceiptIdentity,
  advanceReceiptFence,
  deleteReceipt,
  findReceiptByConversation,
  findReceiptByPlaceholder,
  listDeletionIntents,
  recordCreateReceipt,
  recordDeletionIntent,
  resetAssistantReceiptStoreForTests,
  retireReceiptAfterMaterialization,
} from "./assistant-receipt-store";

const user = (id: string) => ({ id }) as User;

describe("assistant receipt store", () => {
  beforeEach(() => {
    useAuthStore.getState().setUser(null);
    localStorage.clear();
    resetAssistantReceiptStoreForTests();
    useAuthStore.getState().setUser(user("user-a"));
  });

  it("records, adopts, and advances a receipt without decreasing its fence", () => {
    const now = Date.now();
    recordCreateReceipt("command-a", "workflow-pending-a", now);
    adoptReceiptIdentity("command-a", "chatc-a", 3, now + 100);
    advanceReceiptFence("chatc-a", 2, now + 200);

    expect(findReceiptByPlaceholder("workflow-pending-a")).toMatchObject({
      commandId: "command-a",
      conversationId: "chatc-a",
      stateVersion: 3,
      createdAt: now,
      updatedAt: now + 200,
    });
    expect(findReceiptByConversation("chatc-a")?.stateVersion).toBe(3);
  });

  it("deletes a definitive receipt without touching its deletion intent", () => {
    recordCreateReceipt("command-a", "workflow-pending-a");
    recordDeletionIntent("delete-a", "workflow-pending-delete");
    deleteReceipt("command-a");

    expect(findReceiptByPlaceholder("workflow-pending-a")).toBeUndefined();
    expect(listDeletionIntents()).toHaveLength(1);
  });

  it("retires a materialized receipt after its retention floor", async () => {
    vi.useFakeTimers();
    try {
      const now = Date.now();
      recordCreateReceipt("command-a", "workflow-pending-a", now);
      retireReceiptAfterMaterialization("command-a", now);

      await vi.advanceTimersByTimeAsync(59_999);
      expect(findReceiptByPlaceholder("workflow-pending-a")).toBeDefined();
      await vi.advanceTimersByTimeAsync(1);
      expect(findReceiptByPlaceholder("workflow-pending-a")).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it("separates accounts and preserves outgoing deletion intents", () => {
    recordCreateReceipt("command-a", "workflow-pending-a");
    recordDeletionIntent("delete-a", "workflow-pending-delete-a");

    useAuthStore.getState().setUser(user("user-b"));
    expect(findReceiptByPlaceholder("workflow-pending-a")).toBeUndefined();
    expect(listDeletionIntents()).toEqual([]);
    recordCreateReceipt("command-b", "workflow-pending-b");

    useAuthStore.getState().setUser(user("user-a"));
    expect(findReceiptByPlaceholder("workflow-pending-a")).toBeDefined();
    expect(findReceiptByPlaceholder("workflow-pending-b")).toBeUndefined();
    expect(listDeletionIntents()[0]?.commandId).toBe("delete-a");
  });

  it("evicts expired and over-cap receipts independently from intents", () => {
    const now = Date.now();
    recordCreateReceipt("expired", "workflow-pending-expired", now - 86_400_001);
    for (let index = 0; index < 22; index += 1) {
      recordCreateReceipt(
        `command-${String(index)}`,
        `workflow-pending-${String(index)}`,
        now + index,
      );
    }
    recordDeletionIntent("delete-a", "workflow-pending-delete", undefined, now);

    expect(findReceiptByPlaceholder("workflow-pending-expired")).toBeUndefined();
    const persisted = JSON.parse(
      localStorage.getItem("nyxid.assistant_receipts.v1.user-a") ?? "{}",
    ) as { receipts?: Record<string, unknown> };
    expect(Object.keys(persisted.receipts ?? {})).toHaveLength(20);
    expect(listDeletionIntents()).toHaveLength(1);
  });

  it("keeps working in memory when browser storage throws", () => {
    const original = globalThis.localStorage;
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("disabled");
      },
      setItem: () => {
        throw new Error("quota");
      },
    });
    resetAssistantReceiptStoreForTests();

    expect(() =>
      recordCreateReceipt("command-a", "workflow-pending-a"),
    ).not.toThrow();
    expect(findReceiptByPlaceholder("workflow-pending-a")).toBeDefined();
    vi.stubGlobal("localStorage", original);
  });

  it("rehydrates current-account evidence from a storage event", () => {
    const key = "nyxid.assistant_receipts.v1.user-a";
    localStorage.setItem(
      key,
      JSON.stringify({
        version: 1,
        ownerUserId: "user-a",
        receipts: {
          remote: {
            placeholderId: "workflow-pending-remote",
            createdAt: Date.now(),
            updatedAt: Date.now(),
          },
        },
        deletionIntents: {},
      }),
    );
    window.dispatchEvent(new StorageEvent("storage", { key }));

    expect(
      findReceiptByPlaceholder("workflow-pending-remote")?.commandId,
    ).toBe("remote");
  });
});
