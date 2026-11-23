import { z } from "zod";
import { attachmentSchema } from "./assistantTypes";
import { MAX_ASSISTANT_ATTACHMENTS } from "./assistantUploads";
import type { AssistantParams } from "../../app/navigationTypes";

const savedFile = z.object({
  name: z.string(),
  mimeType: z.string().nullish(),
  size: z.number().nullish(),
  path: z.string().optional(),
});
const savedRow = z.object({
  file: savedFile,
  handle: z.string(),
  kind: z.enum(["file", "image", "pasted-text"]),
  item: attachmentSchema.optional(),
  needsReupload: z.boolean().optional(),
});
const draftSchema = z.object({
  conversationId: z.string().optional(),
  keepConversation: z.boolean().optional(),
  model: z.string().optional(),
  discardPending: z.boolean().optional(),
  rows: z.array(savedRow).max(MAX_ASSISTANT_ATTACHMENTS),
  reselect: z.boolean(),
});
export type UploadDraft = z.infer<typeof draftSchema>;

export function uploadCachePrefix(uri: string): string {
  return `${uri.replace(/\/+$/, "")}/`;
}

export function relativeCachePath(
  uri: string,
  prefix: string,
): string | undefined {
  if (!uri.startsWith(prefix)) return undefined;
  const path = uri.slice(prefix.length);
  try {
    const decoded = decodeURIComponent(path);
    if (
      decoded &&
      !/[\\?#\0]/.test(decoded) &&
      decoded.split("/").every((part) => part && part !== "." && part !== "..")
    )
      return path;
  } catch {
    /* Invalid escape in a provider URI. */
  }
  return undefined;
}

export function uploadDraftDestination(
  key: string,
):
  | { name: "GroupChat"; params: { groupId: string } }
  | { name: "Assistant"; params: AssistantParams }
  | undefined {
  if (key.startsWith("crew:") && key.length > 5)
    return { name: "GroupChat", params: { groupId: key.slice(5) } };
  try {
    const parts: unknown = JSON.parse(key);
    if (
      !Array.isArray(parts) ||
      parts[0] !== "private" ||
      !parts.every((part) => typeof part === "string")
    )
      return;
    if (parts.length === 2)
      return {
        name: "Assistant",
        params: { conversationId: parts[1], home: false },
      };
    if (parts.length === 3)
      return {
        name: "Assistant",
        params: {
          agentId: parts[1] === "nyxbot" ? undefined : parts[1],
          draftId: parts[2],
          draft: true,
          home: false,
        },
      };
  } catch {
    /* Home has no separate destination. */
  }
}

type Storage = {
  getItem(key: string): Promise<string | null>;
  setItem(key: string, value: string): Promise<void>;
  removeItem(key: string): Promise<void>;
  getAllKeys(): Promise<readonly string[]>;
  multiRemove(keys: string[]): Promise<void>;
};
export function createUploadDraftRecovery(storage: Storage) {
  const prefix = (owner: string) =>
    "nyxid.upload-draft.v1." + encodeURIComponent(owner) + ":";
  const pending = new Map<string, Promise<void>>();
  const discards = new Map<string, Promise<void>>();
  function ordered<T>(owner: string, operation: () => Promise<T>) {
    const result = (pending.get(owner) ?? Promise.resolve()).then(operation);
    const next = result.then(
      () => {},
      () => {},
    );
    pending.set(owner, next);
    void next.then(() => {
      if (pending.get(owner) === next) pending.delete(owner);
    });
    return result;
  }
  async function read(id: string): Promise<UploadDraft | undefined> {
    const raw = await storage.getItem(id);
    if (!raw) return;
    try {
      return draftSchema.parse(JSON.parse(raw));
    } catch {
      return { rows: [], reselect: true };
    }
  }
  return {
    load(owner: string, key: string) {
      return ordered(owner, () => read(prefix(owner) + key));
    },
    save(owner: string, key: string, draft: UploadDraft, guard?: () => void) {
      const id = prefix(owner) + key;
      if (discards.has(id))
        return Promise.reject(new Error("Attachment draft is pending discard"));
      const value =
        draft.rows.length || draft.reselect || draft.keepConversation || draft.model || draft.discardPending
          ? JSON.stringify(draft)
          : undefined;
      return ordered(owner, async () => {
        guard?.();
        const existing = await read(id);
        guard?.();
        if (existing?.discardPending) {
          if (!draft.discardPending)
            throw new Error("Attachment draft is pending discard");
          // Retained sessions may still hold the original rows on retry.
          // The stored cleanup record owns the remaining work.
          return;
        }
        if (value) await storage.setItem(id, value);
        else await storage.removeItem(id);
      });
    },
    discard(
      owner: string,
      key: string,
      guard: () => void,
      release: (row: UploadDraft["rows"][number], conversationId?: string) => Promise<void>,
    ) {
      const id = prefix(owner) + key;
      const current = discards.get(id);
      if (current) return current;
      const operation = ordered(owner, async () => {
        guard();
        const draft = await read(id);
        guard();
        if (!draft) return;
        const cleanup = {
          conversationId: draft.conversationId,
          rows: draft.rows,
          reselect: false,
          discardPending: true,
        };
        if (!draft.discardPending || draft.model || draft.keepConversation || draft.reselect)
          await storage.setItem(id, JSON.stringify(cleanup));
        for (const [index, row] of cleanup.rows.entries()) {
          guard();
          await release(row, cleanup.conversationId);
          guard();
          // Keep the storage lane until the checkpoint settles, even if the
          // account changes during the write. Only a confirmed row is removed.
          await storage.setItem(id, JSON.stringify({
            ...cleanup,
            rows: cleanup.rows.slice(index + 1),
          }));
        }
        guard();
        await storage.removeItem(id);
      }).finally(() => discards.delete(id));
      discards.set(id, operation);
      return operation;
    },
    other(owner: string) {
      return ordered(owner, async () => {
        const start = prefix(owner);
        for (const key of await storage.getAllKeys()) {
          if (!key.startsWith(start)) continue;
          const target = uploadDraftDestination(key.slice(start.length));
          if (target) return { ...target, key: key.slice(start.length) };
        }
      });
    },
    clear(owner: string, releaseLocal?: (draft: UploadDraft) => void) {
      return ordered(owner, async () => {
        const keys = (await storage.getAllKeys()).filter((key) =>
          key.startsWith(prefix(owner)),
        );
        if (!releaseLocal) {
          if (keys.length) await storage.multiRemove(keys);
          return;
        }
        const cleared: string[] = [];
        for (const key of keys) {
          const draft = await read(key);
          if (!draft) continue;
          if (draft.discardPending) continue;
          try {
            releaseLocal(draft);
            cleared.push(key);
          } catch {
            // Keep a local-only retry record if the OS refused cache removal.
            await storage.setItem(key, JSON.stringify({
              rows: draft.rows.filter((row) => row.file.path).map(({ file, handle, kind }) => ({
                file, handle, kind,
              })),
              reselect: false,
              discardPending: true,
            } satisfies UploadDraft));
          }
        }
        if (cleared.length) await storage.multiRemove(cleared);
      });
    },
  };
}
