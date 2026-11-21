import type { UploadDraft } from "./uploadDraftRecovery";
import { localizedError } from "../../i18n/localizedError";
import { translateApp } from "../../i18n/appTranslation";
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
  /** Local stat passed the size/format checks; independent of the upload queue. */
  previewReady?: boolean;
  resumable?: boolean;
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
  saveDraft?: (session: AssistantUploadSession) => Promise<void>;
};

const INTERRUPTED = "Upload interrupted. Remove the file and choose it again.";

/** Pending files keep one owner/Agent/chat generation and one durable draft. */
export class AssistantUploadSession {
  readonly entries = new Map<string, PendingAssistantUpload>();
  notice?: string;
  sending = false;
  restoring = false;
  restored = false;
  reselect = false;
  picking = false;
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
    return (
      this.restoring ||
      [...this.entries.values()].some((row) => !row.item || row.error)
    );
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
  snapshot(path: (uri: string) => string | undefined): UploadDraft {
    return {
      conversationId: this.id,
      reselect: this.reselect || this.picking,
      rows: [...this.entries.values()].map(({ file, handle, kind, item }) => ({
        file: {
          name: file.name,
          mimeType: file.mimeType,
          size: file.size,
          path: path(file.uri),
        },
        handle,
        kind,
        item,
      })),
    };
  }
  restore(draft: UploadDraft, resolve: (path?: string) => string | undefined) {
    if (this.id && draft.conversationId && this.id !== draft.conversationId) {
      this.reselect = true;
      return;
    }
    this.id ??= draft.conversationId;
    this.reselect = draft.reselect;
    for (const saved of draft.rows) {
      const uri = resolve(saved.file.path);
      const item = saved.item?.expired ? undefined : saved.item;
      if (!uri && !item) {
        this.reselect = true;
        continue;
      }
      const { name, size, mimeType } = saved.file;
      const file = { name, size, mimeType, uri: uri ?? "" };
      const error = attachmentFileError(file);
      const key = String(++this.sequence);
      const row: PendingAssistantUpload = {
        key,
        handle: saved.handle,
        kind: saved.kind,
        file,
        item,
        progress: item ? 100 : 0,
        error: item ? undefined : (error ?? undefined),
        resumable: !item && !error,
        controller: new AbortController(),
      };
      this.entries.set(key, row);
      if (uri && !error) {
        // An evicted or changed cache entry does not invalidate a saved receipt.
        void this.validatePreview(row).catch(() => undefined);
      }
    }
  }
  async saveDraft() {
    if (!this.restoring && this.alive()) await this.deps.saveDraft?.(this);
  }
  private checkpoint() {
    if (!this.deps.saveDraft) return;
    void this.saveDraft().catch(() => {
      if (this.alive()) {
        this.notice = translateApp(
          "These attachments could not be saved on this device. Keep this chat open.",
        );
        this.notify();
      }
    });
  }
  dismissReselection() {
    this.reselect = false;
    this.checkpoint();
    this.notify();
  }
  resume() {
    if (!this.available() || this.sending) return;
    for (const row of this.entries.values()) {
      if (!row.resumable) continue;
      row.resumable = false;
      this.enqueue(row);
    }
    this.notify();
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
      throw localizedError("This composer is no longer available.");
    const id = await this.ensureDraft();
    if (!this.available())
      throw localizedError("This composer is no longer available.");
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
    for (const row of this.entries.values())
      reserved.add(row.handle.toLowerCase());
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
        this.notice = translateApp(
          "Up to 10 files fit in one message. Send the remaining files in another message.",
        );
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
      this.enqueue(row);
    }
    this.checkpoint();
    this.notify();
  }
  private async validatePreview(row: PendingAssistantUpload) {
    if (!this.usable(row)) return;
    const size = await this.deps.fileSize(row.file);
    const error = attachmentFileError(row.file, size);
    if (error) throw new Error(error);
    if (!this.usable(row)) return;
    row.previewReady = true;
    this.notify();
  }
  private enqueue(row: PendingAssistantUpload) {
    const file = row.file;
    // Validate previews independently. Only the upload queue reads file bytes.
    const prepared = this.validatePreview(row).catch((cause) => {
      if (this.alive() && this.entries.get(row.key) === row && !row.error) {
        row.previewReady = false;
        row.error = attachmentUploadError(cause);
        this.notify();
      }
    });
    this.queue = this.queue.then(async () => {
      try {
        await prepared;
        if (!this.usable(row) || row.error) return;
        let bytes: Uint8Array;
        try {
          bytes = await this.deps.readBytes(file, row.controller.signal);
          const error = attachmentFileError(file, bytes.byteLength);
          if (error) throw new Error(error);
        } catch (cause) {
          row.previewReady = false;
          throw cause;
        }
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
          row.error = translateApp(INTERRUPTED);
        this.checkpoint();
        this.notify();
      }
    });
  }
  remove(key: string) {
    if (this.sending) return;
    const row = this.entries.get(key);
    if (!row) return;
    this.entries.delete(key);
    row.controller.abort();
    if (row.item && this.id) this.removeRemote(this.id, row.item.id);
    this.disposeFile(row.file);
    this.checkpoint();
    this.notify();
  }
  interrupt() {
    for (const row of this.entries.values()) {
      if (!row.item && !row.error && !row.resumable) {
        row.controller.abort();
        row.error = translateApp(INTERRUPTED);
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
      throw localizedError("Reconnect and keep this chat open to send.");
    if (this.sending) throw localizedError("A message is already being sent.");
    if (this.blocked)
      throw localizedError(
        "Wait for uploads to finish or remove failed files.",
      );
    const rows = [...this.entries.values()];
    const reselect = this.reselect;
    if (rows.length && !this.id)
      throw localizedError("The upload draft is unavailable.");
    this.sending = true;
    this.notify();
    let detached = false;
    let restore = true;
    try {
      if (!this.available())
        throw localizedError("Reconnect and keep this chat open to send.");
      const payload = rows.length
        ? {
            conversationId: this.id!,
            attachmentIds: rows.map((row) => row.item!.id),
            attachments: rows.map((row) => row.item!),
          }
        : undefined;
      this.entries.clear();
      this.reselect = false;
      for (const row of rows) this.detached.add(row);
      detached = true;
      this.notify();
      if ((rows.length || reselect) && this.deps.saveDraft) {
        // A process exit after dispatch must never resurrect potentially claimed files.
        await this.saveDraft();
        if (!this.available())
          throw localizedError("Reconnect and keep this chat open to send.");
      }
      const accepted = await send(payload);
      restore = !accepted;
      return accepted;
    } finally {
      if (detached) {
        if (restore && this.alive()) {
          this.reselect = reselect;
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
      if (rows.length || reselect) this.checkpoint();
      this.notify();
    }
  }
}
