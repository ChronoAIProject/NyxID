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
});
const draftSchema = z.object({
  conversationId: z.string().optional(),
  rows: z.array(savedRow).max(MAX_ASSISTANT_ATTACHMENTS),
  reselect: z.boolean(),
});
export type UploadDraft = z.infer<typeof draftSchema>;

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
  let pending = Promise.resolve();
  function ordered<T>(operation: () => Promise<T>) {
    const result = pending.then(operation);
    pending = result.then(
      () => {},
      () => {},
    );
    return result;
  }
  return {
    load(owner: string, key: string) {
      return ordered(async () => {
        const raw = await storage.getItem(prefix(owner) + key);
        if (!raw) return;
        try {
          return draftSchema.parse(JSON.parse(raw));
        } catch {
          return { rows: [], reselect: true } satisfies UploadDraft;
        }
      });
    },
    save(owner: string, key: string, draft: UploadDraft) {
      const id = prefix(owner) + key;
      const value =
        draft.rows.length || draft.reselect ? JSON.stringify(draft) : undefined;
      return ordered(() =>
        value ? storage.setItem(id, value) : storage.removeItem(id),
      );
    },
    other(owner: string) {
      return ordered(async () => {
        const start = prefix(owner);
        for (const key of await storage.getAllKeys()) {
          if (!key.startsWith(start)) continue;
          const target = uploadDraftDestination(key.slice(start.length));
          if (target) return { ...target, key: key.slice(start.length) };
        }
      });
    },
    clear(owner: string) {
      return ordered(async () => {
        const keys = (await storage.getAllKeys()).filter((key) =>
          key.startsWith(prefix(owner)),
        );
        if (keys.length) await storage.multiRemove(keys);
      });
    },
  };
}
