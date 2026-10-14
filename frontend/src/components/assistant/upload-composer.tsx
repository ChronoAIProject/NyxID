import { useEffect, useRef, useState, type ComponentProps } from "react";
import { FileText, Paperclip, X } from "lucide-react";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogFooter,
  DialogTitle,
} from "@/components/ui/dialog";
import { ChatComposer } from "@/components/assistant/chat-composer";
import {
  createUploadDraft,
  isSupportedAttachment,
  removeUpload,
  UNSUPPORTED_FORMAT_MESSAGE,
  uploadFile,
  type UploadedMessage,
  type UploadScope,
} from "@/lib/assistant/uploads";
import {
  attachmentKind,
  draftWithoutHandles,
  pastedTextFile,
  resolveAttachmentMentions,
  type AttachmentKind,
} from "@/lib/assistant/attachment-mentions";
import { isAdmittedSendError } from "@/lib/assistant/send-admission";
import type { NyxAgentAttachment } from "@/schemas/assistant-nyxagent";

type Pending = {
  key: string;
  /** What the writer types after `@` to mention this file, e.g. `image-1`. */
  handle: string;
  kind: AttachmentKind;
  /** A long paste's own text, so a saved draft can restore it inline. */
  text?: string;
  name: string;
  preview?: string;
  progress: number;
  error?: string;
  item?: NyxAgentAttachment;
  controller: AbortController;
};
type Props = Omit<ComponentProps<typeof ChatComposer>, "onSend" | "onVoice"> & {
  readonly scope: UploadScope;
  readonly onVoice?: (conversationId: string) => Promise<void>;
  readonly onSend: (text: string, uploads?: UploadedMessage) => Promise<void>;
};

/** Keyed by owner + conversation at the call site: drafts never cross threads. */
export function UploadComposer({ scope, onSend, onVoice, ...props }: Props) {
  const [files, setFiles] = useState<Pending[]>([]);
  const [notice, setNotice] = useState<string>();
  const [preview, setPreview] = useState<Pending>();
  const pending = useRef(new Map<string, Pending>());
  const input = useRef<HTMLInputElement>(null);
  const attach = useRef<HTMLButtonElement>(null);
  const uploadQueue = useRef(Promise.resolve());
  const openingVoice = useRef(false);
  const scopeId = useRef<Promise<string> | undefined>(undefined);
  const resolved = useRef<UploadScope>(scope);
  const alive = useRef(true);
  const handleCounts = useRef<Record<AttachmentKind, number>>({ image: 0, file: 0, "pasted-text": 0 });
  const locked = props.disabled || props.active || props.sending;
  useEffect(() => {
    alive.current = true;
    const entries = pending.current;
    return () => {
      alive.current = false;
      for (const file of entries.values()) {
        file.controller.abort();
        if (file.preview) URL.revokeObjectURL(file.preview);
      }
    };
  }, []);

  function refresh() {
    if (alive.current) setFiles([...pending.current.values()]);
  }
  function remove(key: string) {
    const file = pending.current.get(key);
    if (!file) return;
    pending.current.delete(key);
    file.controller.abort();
    if (file.preview) URL.revokeObjectURL(file.preview);
    if (file.item)
      void removeUpload(resolved.current, file.item.id).catch(() => undefined);
    refresh();
    requestAnimationFrame(() => attach.current?.focus());
  }
  /** Queues `incoming` for upload and returns the handles it was given. */
  function add(incoming: File[], kind?: AttachmentKind, text?: string): string[] {
    if (locked) return [];
    setNotice(undefined);
    const handles: string[] = [];
    for (const file of incoming) {
      if (pending.current.size >= 10) {
        setNotice(
          "Up to 10 files fit in one message. Send the remaining files in another message.",
        );
        break;
      }
      const fileKind = kind ?? attachmentKind(file);
      handleCounts.current[fileKind] += 1;
      const entry: Pending = {
        key: crypto.randomUUID(),
        handle: `${fileKind}-${String(handleCounts.current[fileKind])}`,
        kind: fileKind,
        ...(text === undefined ? {} : { text }),
        name: file.name,
        progress: 0,
        controller: new AbortController(),
      };
      handles.push(entry.handle);
      if (!isSupportedAttachment(file)) {
        entry.error = UNSUPPORTED_FORMAT_MESSAGE;
        pending.current.set(entry.key, entry);
        refresh();
        continue;
      }
      if (
        ["image/png", "image/jpeg", "image/gif", "image/webp"].includes(
          file.type,
        ) &&
        file.size <= 20 * 1024 * 1024
      )
        entry.preview = URL.createObjectURL(file);
      pending.current.set(entry.key, entry);
      refresh();
      uploadQueue.current = uploadQueue.current.then(async () => {
        if (!alive.current || entry.controller.signal.aborted) return;
        try {
          if (!file.size || file.size > 20 * 1024 * 1024)
            throw new Error("Choose a file between 1 byte and 20 MB.");
          scopeId.current ??= scope.id
            ? Promise.resolve(scope.id)
            : createUploadDraft(scope.agentId);
          const id = await scopeId.current;
          resolved.current = { ...scope, id };
          if (!alive.current || entry.controller.signal.aborted) return;
          const item = await uploadFile(
            resolved.current,
            file,
            entry.controller.signal,
            (value) => {
              entry.progress = value;
              refresh();
            },
          );
          if (!alive.current || !pending.current.has(entry.key)) {
            void removeUpload(resolved.current, item.id).catch(() => undefined);
            return;
          }
          entry.item = item;
          entry.progress = 100;
        } catch (error) {
          entry.error =
            error instanceof Error ? error.message : "Upload failed.";
          // A failed draft request can be retried by removing and adding again.
          if (!resolved.current.id) scopeId.current = undefined;
        }
        refresh();
      });
    }
    return handles;
  }
  async function openVoice() {
    if (locked || openingVoice.current || pending.current.size || scope.kind !== "conversations" || !onVoice) return;
    openingVoice.current = true;
    try {
      scopeId.current ??= scope.id ? Promise.resolve(scope.id) : createUploadDraft(scope.agentId);
      const id = await scopeId.current;
      resolved.current = { ...scope, id };
      if (alive.current) await onVoice(id);
    } catch {
      if (!resolved.current.id) scopeId.current = undefined;
      if (alive.current) setNotice("Voice could not open. Please try again.");
    } finally { openingVoice.current = false; }
  }
  return (
    <ChatComposer
      {...props}
      onVoice={onVoice ? () => { void openVoice(); } : undefined}
      hasAttachments={files.some((file) => file.item)}
      uploadBlocked={files.some((file) => !file.item) || Boolean(props.active && files.length)}
      onFiles={(items) => void add(items)}
      // Every chip, failed ones included: a reference goes only when its chip is removed.
      attachmentMentions={files.map((file) => ({
        handle: file.handle,
        label: file.item?.label ?? file.name,
        kind: file.kind,
      }))}
      onLongText={(text) => {
        if (locked || pending.current.size >= 10) return undefined;
        // Named like its handle: @pasted-text-2 is pasted-text-2.md.
        const file = pastedTextFile(text, handleCounts.current["pasted-text"] + 1);
        return add([file], "pasted-text", text)[0];
      }}
      draftText={(content) =>
        draftWithoutHandles(
          content,
          [...pending.current.values()].map((file) => ({
            handle: file.handle,
            label: file.item?.label ?? file.name,
            kind: file.kind,
            text: file.text,
          })),
        )
      }
      attachments={
        <div className={files.length || notice ? "mb-2 space-y-2" : undefined}>
          {files.length > 0 && (
            <ul aria-label="Attachments" className="assistant-scrollbar grid max-h-44 gap-1.5 overflow-y-auto overscroll-contain sm:grid-cols-2">
              {files.map((file) => (
                <li
                  key={file.key}
                  className="flex min-w-0 items-center gap-2 rounded-lg border border-hairline bg-card p-2 text-xs"
                >
                  {file.preview ? (
                    <button
                      type="button"
                      className="shrink-0"
                      aria-label={`Preview ${file.name}`}
                      onClick={() => setPreview(file)}
                    >
                      <img
                        src={file.preview}
                        alt={file.name}
                        className="h-16 w-16 shrink-0 rounded object-cover"
                      />
                    </button>
                  ) : (
                    <FileText className="h-5 w-5 shrink-0 text-muted-foreground" />
                  )}
                  <div className="min-w-0 flex-1">
                    <p className="truncate">
                      <span
                        className="mr-1.5 font-mono text-muted-foreground"
                        title={`Type @${file.handle} to refer to this file`}
                      >
                        @{file.handle}
                      </span>
                      {file.item?.label ?? file.name}
                    </p>
                    {file.error ? (
                      <p role="alert" className="break-words text-destructive">
                        {file.error}
                      </p>
                    ) : (
                      <p role="status" className="text-muted-foreground">
                        {file.item
                          ? "Ready"
                          : file.progress >= 99
                            ? "Checking file…"
                            : `Uploading ${file.progress}%`}
                      </p>
                    )}
                  </div>
                  <Button
                    type="button"
                    size="icon"
                    variant="ghost"
                    aria-label={`Remove ${file.name}`}
                    onClick={() => remove(file.key)}
                    disabled={props.sending}
                  >
                    <X className="h-4 w-4" />
                  </Button>
                </li>
              ))}
            </ul>
          )}
          {notice && (
            <p role="status" className="text-xs text-muted-foreground">
              {notice}
            </p>
          )}
          <Dialog
            open={Boolean(preview?.preview)}
            onOpenChange={(open) => {
              if (!open) setPreview(undefined);
            }}
          >
            <DialogContent aria-describedby={undefined} className="md:max-w-3xl">
              <DialogTitle className="truncate pr-8">{preview?.name}</DialogTitle>
              {preview?.preview && (
                <img
                  src={preview.preview}
                  alt={preview.name}
                  className="max-h-[70vh] w-full rounded object-contain"
                />
              )}
              <DialogFooter>
                <DialogClose asChild>
                  <Button type="button" variant="secondary">
                    Close preview
                  </Button>
                </DialogClose>
              </DialogFooter>
            </DialogContent>
          </Dialog>
        </div>
      }
      attachmentButton={
        <>
          {/* No picker MIME filter: it would hide extensionless text and files
              with unusual browser MIME guesses. `add` applies the same allowlist
              to picker, paste and drop, and the server still sniffs every file. */}
          <input
            ref={input}
            type="file"
            multiple
            className="sr-only"
            tabIndex={-1}
            aria-label="Choose attachments"
            disabled={locked}
            onChange={(event) => {
              void add(Array.from(event.target.files ?? []));
              event.target.value = "";
            }}
          />
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  type="button"
                  variant="ghost"
                  size="icon"
                  className="shrink-0 text-muted-foreground"
                  ref={attach}
                  aria-label="Attach files"
                  disabled={locked || files.length >= 10}
                  onClick={() => input.current?.click()}
                >
                  <Paperclip className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent className="max-w-64">
                PNG, JPEG, GIF, WebP, PDF, DOCX, text, Markdown, CSV and JSON.
                Up to 20 MB each, 10 files per message.
              </TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </>
      }
      onSend={async (text) => {
        const items = [...pending.current.values()];
        if (items.some((file) => !file.item)) return;
        // The original send may await the entire streamed reply. Detach its
        // files now so text guidance cannot reattach them or be blocked by them.
        for (const file of items) pending.current.delete(file.key);
        refresh();
        try {
          await onSend(
            resolveAttachmentMentions(
              text,
              items.map((file) => ({ handle: file.handle, label: file.item!.label, kind: file.kind })),
            ),
            items.length
              ? {
                  attachmentIds: items.map((file) => file.item!.id),
                  attachments: items.map((file) => file.item!),
                  conversationId:
                    scope.kind === "conversations"
                      ? resolved.current.id
                      : undefined,
                }
              : undefined,
          );
        } catch (error) {
          // Admitted uploads are bound to the stored message: never offer them again.
          const admitted = isAdmittedSendError(error);
          for (const file of items) {
            if (alive.current && !admitted) pending.current.set(file.key, file);
            else if (file.preview) URL.revokeObjectURL(file.preview);
          }
          refresh();
          throw error;
        }
        for (const file of items) {
          if (file.preview) URL.revokeObjectURL(file.preview);
        }
      }}
    />
  );
}
