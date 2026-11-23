import type { UploadDraft } from "./uploadDraftRecovery";
import { isAttachmentBindRefusal } from "./assistantTurnRequest";
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
  needsReupload?: boolean;
  removing?: boolean;
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
  remove: (id: string, attachmentId: string, signal?: AbortSignal) => Promise<unknown>;
  disposeFile: (file: PickedAssistantFile, strict?: boolean) => void;
  /** Owner and exact composer; availability does not change ownership. */
  current: () => boolean;
  /** Check focus, foreground, connectivity and compose permission live. */
  available: () => boolean;
  /** Live group member names must not collide with attachment references. */
  reservedMentionHandles?: () => readonly string[];
  changed: () => void;
  saveDraft?: (session: AssistantUploadSession) => Promise<void>;
};

const INTERRUPTED = "Upload interrupted. Remove the file and choose it again.";
const UNAVAILABLE = "This file is no longer available. Choose it again.";
const DISCARD_FAILED = "The draft could not be discarded. Try again.";

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
  discardPending = false;
  private sequence = 0;
  private mentionSequence: Record<AttachmentKind, number> = {
    image: 0,
    file: 0,
    "pasted-text": 0,
  };
  private queue = Promise.resolve();
  private writes = Promise.resolve();
  private draft?: Promise<string>;
  private id?: string;
  private readonly ownsDraft: boolean;
  private draftCheckpointed = false;
  private draftCheckpointFailed = false;
  private consumed = false;
  private modelSaving = false;
  model?: string;
  private controller = new AbortController();
  private detached = new Set<PendingAssistantUpload>();
  private listeners = new Set<() => void>();
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
      this.modelSaving ||
      this.discardPending ||
      [...this.entries.values()].some(
        (row) => !row.item || row.error || row.needsReupload || row.removing,
      )
    );
  }
  get conversationId() {
    return this.id;
  }
  get hasDraft() {
    return this.ownsDraft && !this.consumed && (!!this.model || !!this.id);
  }
  /** A retained composer reattaches its current lifecycle and account guards. */
  rebind(deps: Dependencies) {
    this.deps = deps;
  }
  private alive() {
    return !this.disposed && this.deps.current();
  }
  private available() {
    return this.alive() && !this.discardPending && this.deps.available();
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
    for (const listener of this.listeners) listener();
  }
  subscribe(listener: () => void) {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }
  finishRestore() {
    this.restoring = false;
    this.restored = true;
    this.notify();
  }
  snapshot(path: (uri: string) => string | undefined): UploadDraft {
    return {
      conversationId: this.id,
      keepConversation: !this.discardPending && this.hasDraft && !!this.id,
      model: this.consumed || this.discardPending ? undefined : this.model,
      discardPending: this.discardPending,
      reselect: !this.discardPending && (this.reselect || this.picking),
      rows: [...this.entries.values()].map(({ file, handle, kind, item, needsReupload }) => ({
        file: {
          name: file.name,
          mimeType: file.mimeType,
          size: file.size,
          path: path(file.uri),
        },
        handle,
        kind,
        item,
        needsReupload,
      })),
    };
  }
  restore(draft: UploadDraft, resolve: (path?: string) => string | undefined) {
    if (this.id && draft.conversationId && this.id !== draft.conversationId) {
      this.reselect = true;
      return;
    }
    this.id ??= draft.conversationId;
    this.draftCheckpointed = !!this.id;
    this.reselect = draft.reselect;
    this.discardPending ||= draft.discardPending === true;
    this.model = this.discardPending ? undefined : draft.model;
    if (this.discardPending) this.notice = translateApp(DISCARD_FAILED);
    for (const saved of draft.rows) {
      const uri = resolve(saved.file.path);
      const item = saved.item;
      const needsReupload = saved.needsReupload || item?.expired === true;
      const ready = item && !needsReupload;
      if (!uri && !item) {
        this.reselect = true;
        continue;
      }
      const { name, size, mimeType } = saved.file;
      const file = { name, size, mimeType, uri: uri ?? "" };
      const error = uri ? attachmentFileError(file) : translateApp(UNAVAILABLE);
      const key = String(++this.sequence);
      const row: PendingAssistantUpload = {
        key,
        handle: saved.handle,
        kind: saved.kind,
        file,
        item,
        needsReupload,
        progress: ready ? 100 : 0,
        error: ready ? undefined : (error ?? undefined),
        resumable: !ready && !error,
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
    await this.persistDraft(false);
  }
  private async persistDraft(preparingDiscard: boolean) {
    if (this.restoring || this.disposed || !this.deps.saveDraft) return;
    await this.writeDraft(async () => {
      // Prepare the cleanup record only after queued uploads have stopped.
      if (!this.restoring && !this.disposed && this.discardPending === preparingDiscard)
        await this.deps.saveDraft?.(this);
    });
  }
  private writeDraft(write: () => Promise<void>) {
    const operation = this.writes.then(write);
    this.writes = operation.catch(() => {
      // Keep the lane available after a failed write. The caller handles it.
    });
    return operation;
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
    if (this.discardPending) return;
    this.reselect = false;
    this.checkpoint();
    this.notify();
  }
  resume() {
    if (!this.available() || this.sending) return;
    this.draftCheckpointFailed = false;
    for (const row of this.entries.values()) {
      if (!row.resumable || row.removing) continue;
      row.resumable = false;
      row.error = undefined;
      row.controller = new AbortController();
      this.enqueue(row);
    }
    this.notify();
  }
  private disposeFile(file: PickedAssistantFile, strict = false) {
    if (this.released.has(file)) return;
    this.deps.disposeFile(file, strict);
    this.released.add(file);
  }
  private removeRemote(id: string, attachmentId: string) {
    // The native adapter independently fences cleanup to the original owner.
    void this.deps.remove(id, attachmentId).catch(() => undefined);
  }
  /** Details can edit this exact draft while its composer is unfocused. */
  async selectModel(model: string) {
    if (!this.ownsDraft || this.consumed || this.disposed ||
        this.restoring || this.discardPending || this.sending || this.modelSaving)
      throw localizedError("This composer is no longer available.");
    this.modelSaving = true;
    try {
      await this.writeDraft(async () => {
        if (this.disposed || this.discardPending)
          throw localizedError("This composer is no longer available.");
        const previous = this.model;
        this.model = model;
        try {
          await this.deps.saveDraft?.(this);
        } catch (cause) {
          this.model = previous;
          throw cause;
        }
      });
    } finally {
      this.modelSaving = false;
      this.notify();
    }
  }
  async applyDraftModel(
    id: string,
    change: (id: string, model: string) => Promise<unknown>,
  ) {
    if (!this.available() || !this.ownsDraft || this.id !== id)
      throw localizedError("This composer is no longer available.");
    if (this.model) await change(id, this.model);
    if (!this.available())
      throw localizedError("This composer is no longer available.");
  }
  /** Voice opens the saved chat after its draft identity and model are confirmed. */
  async consumeDraft() {
    this.consumed = true;
    try {
      await this.saveDraft();
    } catch (cause) {
      this.consumed = false;
      throw cause;
    }
  }
  /** Voice and attachments share the same durable unsent private thread. */
  async ensureConversation(createDraft = this.deps.createDraft) {
    if (!this.available() || this.restoring)
      throw localizedError("This composer is no longer available.");
    const id = await this.ensureDraft(createDraft);
    if (!this.available())
      throw localizedError("This composer is no longer available.");
    return id;
  }
  private async ensureDraft(createDraft = this.deps.createDraft) {
    if (this.id && (!this.ownsDraft || this.draftCheckpointed)) return this.id;
    this.draft ??= (async () => {
      this.id ??= await createDraft(this.controller.signal);
      if (this.disposed || this.discardPending)
        throw localizedError("This composer is no longer available.");
      // The server has committed. Store this ID before any request can use it.
      // A storage failure retains the ID here so Retry does not create again.
      try {
        await this.saveDraft();
      } catch (cause) {
        this.draftCheckpointFailed = true;
        throw cause;
      }
      this.draftCheckpointFailed = false;
      this.draftCheckpointed = true;
      return this.id;
    })();
    const pending = this.draft;
    try {
      return await pending;
    } finally {
      if (this.draft === pending) this.draft = undefined;
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
    if (!this.usable(row) || row.error) return;
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
      let checkpointing = false;
      let releasingReceipt = false;
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
        checkpointing = true;
        if (this.draftCheckpointFailed)
          throw localizedError("These attachments could not be saved on this device. Keep this chat open.");
        const id = await this.ensureDraft();
        checkpointing = false;
        if (!this.usable(row)) return;
        if (row.needsReupload && row.item) {
          releasingReceipt = true;
          await this.deps.remove(id, row.item.id, this.controller.signal);
          row.item = undefined;
          releasingReceipt = false;
        }
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
        if (this.disposed || this.entries.get(row.key) !== row ||
            !this.deps.saveDraft && !this.usable(row)) {
          this.removeRemote(id, item.id);
          return;
        }
        row.item = item;
        row.needsReupload = false;
        row.error = undefined;
        row.progress = 100;
      } catch (cause) {
        if (this.entries.get(row.key) === row && !row.error) {
          row.error = attachmentUploadError(cause);
          row.resumable = (checkpointing && !!this.id) || releasingReceipt;
        }
      } finally {
        // React's interrupt effect may follow an async availability change.
        if (
          this.alive() &&
          this.entries.get(row.key) === row &&
          (!row.item || row.needsReupload) &&
          !row.error
        )
          row.error = translateApp(INTERRUPTED);
        this.checkpoint();
        this.notify();
      }
    });
  }
  remove(key: string) {
    if (this.sending || this.discardPending) return;
    const row = this.entries.get(key);
    if (!row || row.removing) return;
    if (this.deps.saveDraft) {
      row.controller.abort();
      row.removing = true;
      this.notify();
      this.queue = this.queue.then(async () => {
        if (this.disposed) return;
        try {
          if (row.item && this.id)
            await this.deps.remove(this.id, row.item.id, this.controller.signal);
          if (this.disposed || this.discardPending) return;
          this.disposeFile(row.file, true);
          this.entries.delete(key);
          await this.saveDraft();
          this.notice = undefined;
        } catch {
          this.entries.set(key, row);
          row.error = translateApp("The attachment could not be removed. Try again.");
          this.notice = row.error;
        } finally {
          row.removing = false;
          this.notify();
        }
      });
      return;
    }
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
  /** Stop retained work without deleting recoverable files on account change. */
  suspend(clearLocal = false) {
    this.disposed = true;
    this.controller.abort();
    for (const row of this.entries.values()) {
      row.controller.abort();
      if (clearLocal) this.disposeFile(row.file);
    }
    for (const row of this.detached) this.disposeFile(row.file);
    this.notify();
  }
  async prepareDiscard() {
    if (this.sending || this.picking)
      throw localizedError(DISCARD_FAILED);
    this.discardPending = true;
    this.controller.abort();
    this.interrupt();
    await this.queue;
    await this.draft?.catch(() => undefined);
    this.notice = translateApp(DISCARD_FAILED);
    this.notify();
    await this.persistDraft(true);
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
      if (this.id && this.ownsDraft) await this.ensureDraft();
      const payload = this.id && (rows.length || this.ownsDraft)
        ? {
            conversationId: this.id,
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
      if (accepted) this.consumed = true;
      return accepted;
    } catch (cause) {
      if (!rows.length || !isAttachmentBindRefusal(cause)) throw cause;
      for (const row of rows) {
        row.needsReupload = true;
        row.progress = 0;
        row.resumable = !!row.file.uri;
        row.error = row.resumable ? undefined : translateApp(UNAVAILABLE);
      }
      this.notice = translateApp(
        "One or more attachments are no longer available. Resume uploads or choose the files again.",
      );
      return false;
    } finally {
      if (detached) {
        if (restore && !this.disposed) {
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
      if (rows.length || reselect || this.ownsDraft && (this.id || this.model)) {
        // A definite bind refusal must durably invalidate the old receipt
        // before another send. An interrupted write remains safe because the
        // pre-dispatch checkpoint removed the submitted rows.
        await this.saveDraft().catch(() => {
          this.notice = translateApp(
            "These attachments could not be saved on this device. Keep this chat open.",
          );
        });
      }
      this.notify();
    }
  }
}
