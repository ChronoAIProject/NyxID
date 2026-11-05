import type { AssistantAttachment } from "./assistantTypes";
import { attachmentKind, type AttachmentKind } from "./assistantMentions";
import {
  attachmentFileError,
  attachmentUploadError,
  MAX_ASSISTANT_ATTACHMENTS,
  type AssistantUploadFile,
} from "./assistantUploads";

export type PickedAssistantFile = AssistantUploadFile & { uri: string };
export type PendingAssistantUpload = {
  key: string;
  /** Stable composer-local reference, independent of the numeric row key. */
  handle: string;
  kind: AttachmentKind;
  file: PickedAssistantFile;
  progress: number;
  item?: AssistantAttachment;
  error?: string;
  controller: AbortController;
};
export type AssistantUploadedMessage = {
  conversationId: string;
  attachmentIds: string[];
  attachments: AssistantAttachment[];
};
type Dependencies = {
  createDraft: (signal: AbortSignal) => Promise<string>;
  /** Stat before loading bytes, even when the picker did not report size. */
  fileSize: (file: PickedAssistantFile) => number | Promise<number>;
  readBytes: (
    file: PickedAssistantFile,
    signal: AbortSignal,
  ) => Promise<Uint8Array>;
  upload: (
    id: string,
    file: PickedAssistantFile,
    bytes: Uint8Array,
    signal: AbortSignal,
    progress: (value: number) => void,
  ) => Promise<AssistantAttachment>;
  remove: (id: string, attachmentId: string) => Promise<unknown>;
  disposeFile: (file: PickedAssistantFile) => void;
  /** Owner + Agent + chat generation; availability does not change ownership. */
  current: () => boolean;
  /** Check focus, foreground, connectivity and compose permission live. */
  available: () => boolean;
  /** Live group member names must not collide with attachment references. */
  reservedMentionHandles?: () => readonly string[];
  changed: () => void;
};

const INTERRUPTED = "Upload interrupted. Remove the file and choose it again.";

/** Pending files keep one owner/Agent/chat generation and one durable draft. */
export class AssistantUploadSession {
  readonly entries = new Map<string, PendingAssistantUpload>();
  notice?: string;
  sending = false;
  disposed = false;
  private sequence = 0;
  private mentionSequence: Record<AttachmentKind, number> = {
    image: 0,
    file: 0,
    "pasted-text": 0,
  };
  private queue = Promise.resolve();
  private draft?: Promise<string>;
  private id?: string;
  private readonly ownsDraft: boolean;
  private controller = new AbortController();
  private detached = new Set<PendingAssistantUpload>();
  private released = new WeakSet<PickedAssistantFile>();

  constructor(
    private deps: Dependencies,
    conversationId?: string,
  ) {
    this.id = conversationId;
    this.ownsDraft = !conversationId;
  }
  get blocked() {
    return [...this.entries.values()].some((row) => !row.item || row.error);
  }
  /** A retained composer reattaches its current lifecycle and account guards. */
  rebind(deps: Dependencies) {
    this.deps = deps;
  }
  private alive() {
    return !this.disposed && this.deps.current();
  }
  private available() {
    return this.alive() && this.deps.available();
  }
  private usable(row: PendingAssistantUpload) {
    return (
      this.available() &&
      this.entries.get(row.key) === row &&
      !row.controller.signal.aborted
    );
  }
  private notify() {
    if (this.alive()) this.deps.changed();
  }
  private disposeFile(file: PickedAssistantFile) {
    if (this.released.has(file)) return;
    this.released.add(file);
    this.deps.disposeFile(file);
  }
  private removeRemote(id: string, attachmentId: string) {
    // The native adapter independently fences cleanup to the original owner.
    void this.deps.remove(id, attachmentId).catch(() => undefined);
  }
  /** Dictation and attachments share the same durable unsent private thread. */
  async ensureConversation() {
    if (!this.available())
      throw new Error("This composer is no longer available.");
    const id = await this.ensureDraft();
    if (!this.available())
      throw new Error("This composer is no longer available.");
    return id;
  }
  private async ensureDraft() {
    if (this.id) return this.id;
    this.draft ??= this.deps.createDraft(this.controller.signal);
    const pending = this.draft;
    try {
      const id = await pending;
      if (this.alive()) this.id = id;
      return id;
    } catch (cause) {
      if (this.draft === pending) this.draft = undefined;
      throw cause;
    }
  }
  private nextHandle(kind: AttachmentKind) {
    const reserved = new Set(
      (this.deps.reservedMentionHandles?.() ?? []).map((name) =>
        name.toLowerCase(),
      ),
    );
    const prefix = kind === "pasted-text" ? "text" : kind;
    let handle: string;
    do {
      handle = `${prefix}-${++this.mentionSequence[kind]}`;
    } while (reserved.has(handle.toLowerCase()));
    return handle;
  }
  add(files: PickedAssistantFile[]) {
    if (!this.available() || this.sending) {
      files.forEach((file) => this.disposeFile(file));
      return;
    }
    this.notice = undefined;
    for (const file of files) {
      if (this.entries.size >= MAX_ASSISTANT_ATTACHMENTS) {
        this.notice =
          "Up to 10 files fit in one message. Send the remaining files in another message.";
        this.disposeFile(file);
        continue;
      }
      const kind = attachmentKind({ type: file.mimeType ?? "" });
      const row: PendingAssistantUpload = {
        key: String(++this.sequence),
        handle: this.nextHandle(kind),
        kind,
        file,
        progress: 0,
        controller: new AbortController(),
        error: attachmentFileError(file) ?? undefined,
      };
      this.entries.set(row.key, row);
      if (row.error) continue;
      this.queue = this.queue.then(async () => {
        try {
          if (!this.usable(row)) return;
          const size = await this.deps.fileSize(file);
          const statError = attachmentFileError(file, size);
          if (statError) throw new Error(statError);
          if (!this.usable(row)) return;
          const bytes = await this.deps.readBytes(file, row.controller.signal);
          const byteError = attachmentFileError(file, bytes.byteLength);
          if (byteError) throw new Error(byteError);
          if (!this.usable(row)) return;
          const id = await this.ensureDraft();
          if (!this.usable(row)) return;
          const item = await this.deps.upload(
            id,
            file,
            bytes,
            row.controller.signal,
            (value) => {
              if (!this.usable(row)) return;
              row.progress = Math.max(0, Math.min(99, Math.round(value)));
              this.notify();
            },
          );
          if (!this.usable(row)) {
            this.removeRemote(id, item.id);
            return;
          }
          row.item = item;
          row.progress = 100;
        } catch (cause) {
          if (this.alive() && this.entries.get(row.key) === row && !row.error)
            row.error = attachmentUploadError(cause);
        } finally {
          // React's interrupt effect may follow an async availability change.
          if (
            this.alive() &&
            this.entries.get(row.key) === row &&
            !row.item &&
            !row.error
          )
            row.error = INTERRUPTED;
          this.notify();
        }
      });
    }
    this.notify();
  }
  remove(key: string) {
    if (this.sending) return;
    const row = this.entries.get(key);
    if (!row) return;
    this.entries.delete(key);
    row.controller.abort();
    if (row.item && this.id) this.removeRemote(this.id, row.item.id);
    this.disposeFile(row.file);
    this.notify();
  }
  interrupt() {
    for (const row of this.entries.values()) {
      if (!row.item && !row.error) {
        row.controller.abort();
        row.error = INTERRUPTED;
      }
    }
    this.notify();
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.controller.abort();
    for (const row of this.entries.values()) {
      row.controller.abort();
      if (row.item && this.id) this.removeRemote(this.id, row.item.id);
      this.disposeFile(row.file);
    }
    this.entries.clear();
    // Submitted uploads may be claimed: release only their local cache.
    for (const row of this.detached) this.disposeFile(row.file);
  }
  /** True consumes uploads, including uncertain delivery; false is a refusal.
   * The send hook only throws for local guards before dispatch. */
  async submit(
    send: (uploads?: AssistantUploadedMessage) => Promise<boolean>,
  ): Promise<boolean> {
    if (!this.available())
      throw new Error("Reconnect and keep this chat open to send.");
    if (this.sending) throw new Error("A message is already being sent.");
    if (this.blocked)
      throw new Error("Wait for uploads to finish or remove failed files.");
    const rows = [...this.entries.values()];
    if (rows.length && !this.id)
      throw new Error("The upload draft is unavailable.");
    this.sending = true;
    this.notify();
    let detached = false;
    let restore = true;
    try {
      if (!this.available())
        throw new Error("Reconnect and keep this chat open to send.");
      const payload = rows.length
        ? {
            conversationId: this.id!,
            attachmentIds: rows.map((row) => row.item!.id),
            attachments: rows.map((row) => row.item!),
          }
        : undefined;
      this.entries.clear();
      for (const row of rows) this.detached.add(row);
      detached = true;
      this.notify();
      const accepted = await send(payload);
      restore = !accepted;
      return accepted;
    } finally {
      if (detached) {
        if (restore && this.alive()) {
          for (const row of rows) this.entries.set(row.key, row);
        } else {
          for (const row of rows) {
            if (restore && this.id) this.removeRemote(this.id, row.item!.id);
            this.disposeFile(row.file);
          }
        }
        for (const row of rows) this.detached.delete(row);
      }
      this.sending = false;
      this.notify();
    }
  }
}
