import { sha256 } from "@noble/hashes/sha2.js";
import type { NyxBotHandoff } from "@/schemas/nyxbot-handoff";
import { subscribeAssistantIdentity } from "./identity";

type Receipt =
  | { readonly state: "pending" | "accepted" }
  | { readonly state: "started"; readonly conversationId: string };
const STORAGE = "nyxid.assistant_handoffs.v1";
const receipts = new Map<string, Receipt>();
const listeners = new Set<() => void>();
let loaded = false;

function restore() {
  if (loaded) return;
  loaded = true;
  try {
    const rows: unknown = JSON.parse(localStorage.getItem(STORAGE) ?? "[]");
    if (!Array.isArray(rows)) return;
    for (const entry of rows.slice(-200)) {
      if (
        !Array.isArray(entry) ||
        typeof entry[0] !== "string" ||
        !/^[a-f0-9]{64}$/.test(entry[0])
      )
        continue;
      const row = entry[1] as Partial<Receipt> | undefined;
      if (
        row?.state === "started" &&
        "conversationId" in row &&
        typeof row.conversationId === "string" &&
        /^nyxa-[a-f0-9]{32}$/.test(row.conversationId)
      ) {
        receipts.set(entry[0], {
          state: "started",
          conversationId: row.conversationId,
        });
      } else if (row?.state === "pending" || row?.state === "accepted") {
        // Reloading cannot prove whether an interrupted POST was admitted.
        receipts.set(entry[0], { state: "accepted" });
      }
    }
  } catch {
    /* Memory receipts still prevent replay when storage is unavailable. */
  }
}

/** Only hashes and outcome IDs reach storage; proposed tasks remain in the transcript. */
export function handoffReceiptKey(
  owner: string,
  conversation: string,
  message: string,
  handoff: NyxBotHandoff,
): string {
  return Array.from(
    sha256(
      new TextEncoder().encode(
        JSON.stringify([owner, conversation, message, handoff]),
      ),
    ),
    (byte) => byte.toString(16).padStart(2, "0"),
  ).join("");
}

export function handoffReceipt(key: string | undefined): Receipt | undefined {
  restore();
  return key ? receipts.get(key) : undefined;
}

export function saveHandoffReceipt(key: string, receipt: Receipt | undefined) {
  restore();
  if (receipt) receipts.set(key, receipt);
  else receipts.delete(key);
  while (receipts.size > 200) receipts.delete(receipts.keys().next().value!);
  try {
    localStorage.setItem(STORAGE, JSON.stringify([...receipts]));
  } catch {
    /* Keep memory receipts. */
  }
  listeners.forEach((listener) => listener());
}

export function subscribeHandoffReceipts(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

subscribeAssistantIdentity(() => {
  receipts.clear();
  loaded = false;
  listeners.forEach((listener) => listener());
});

if (typeof window !== "undefined")
  window.addEventListener("storage", (event) => {
    if (event.key !== STORAGE) return;
    receipts.clear();
    loaded = false;
    listeners.forEach((listener) => listener());
  });
