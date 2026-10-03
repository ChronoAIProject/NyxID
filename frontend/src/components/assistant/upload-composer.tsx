import { useEffect, useRef, useState, type ComponentProps } from "react";
import { FileText, Paperclip, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ChatComposer } from "@/components/assistant/chat-composer";
import {
  UPLOAD_ACCEPT,
  createUploadDraft,
  removeUpload,
  uploadFile,
  type UploadedMessage,
  type UploadScope,
} from "@/lib/assistant/uploads";
import type { NyxAgentAttachment } from "@/schemas/assistant-nyxagent";

type Pending = {
  key: string;
  name: string;
  preview?: string;
  progress: number;
  error?: string;
  item?: NyxAgentAttachment;
  controller: AbortController;
};
type Props = Omit<ComponentProps<typeof ChatComposer>, "onSend"> & {
  readonly scope: UploadScope;
  readonly onSend: (text: string, uploads?: UploadedMessage) => Promise<void>;
};

/** Keyed by owner + conversation at the call site: drafts never cross threads. */
export function UploadComposer({ scope, onSend, ...props }: Props) {
  const [files, setFiles] = useState<Pending[]>([]);
  const [notice, setNotice] = useState<string>();
  const pending = useRef(new Map<string, Pending>());
  const input = useRef<HTMLInputElement>(null);
  const uploadQueue = useRef(Promise.resolve());
  const scopeId = useRef<Promise<string> | undefined>(undefined);
  const resolved = useRef<UploadScope>(scope);
  const alive = useRef(true);
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
  }
  async function add(incoming: File[]) {
    if (locked) return;
    setNotice(undefined);
    for (const file of incoming) {
      if (pending.current.size >= 10) {
        setNotice(
          "Up to 10 files fit in one message. Send the remaining files in another message.",
        );
        break;
      }
      const entry: Pending = {
        key: crypto.randomUUID(),
        name: file.name,
        progress: 0,
        controller: new AbortController(),
      };
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
  }
  return (
    <ChatComposer
      {...props}
      hasAttachments={files.some((file) => file.item)}
      uploadBlocked={files.some((file) => !file.item)}
      onFiles={(items) => void add(items)}
      controls={
        <div className="ml-[30px] mb-2 space-y-2">
          {files.length > 0 && (
            <ul aria-label="Attachments" className="grid gap-2 sm:grid-cols-2">
              {files.map((file) => (
                <li
                  key={file.key}
                  className="flex min-w-0 items-center gap-2 rounded-lg border border-hairline bg-card p-2 text-xs"
                >
                  {file.preview ? (
                    <img
                      src={file.preview}
                      alt="Upload preview"
                      className="h-10 w-10 shrink-0 rounded object-cover"
                    />
                  ) : (
                    <FileText className="h-5 w-5 shrink-0 text-muted-foreground" />
                  )}
                  <div className="min-w-0 flex-1">
                    <p className="truncate">{file.item?.label ?? file.name}</p>
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
          <input
            ref={input}
            type="file"
            multiple
            accept={UPLOAD_ACCEPT}
            className="sr-only"
            aria-label="Choose attachments"
            onChange={(event) => {
              void add(Array.from(event.target.files ?? []));
              event.target.value = "";
            }}
          />
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={locked || files.length >= 10}
            onClick={() => input.current?.click()}
          >
            <Paperclip className="mr-1.5 h-3.5 w-3.5" />
            Attach files
          </Button>
          <span className="ml-2 text-[10px] text-muted-foreground">
            Images and documents · 20 MB each · up to 10
          </span>
        </div>
      }
      onSend={async (text) => {
        const items = [...pending.current.values()];
        if (items.some((file) => !file.item)) return;
        await onSend(
          text,
          items.length
            ? {
                attachmentIds: items.map((file) => file.item!.id),
                conversationId:
                  scope.kind === "conversations"
                    ? resolved.current.id
                    : undefined,
              }
            : undefined,
        );
        for (const file of items) {
          if (file.preview) URL.revokeObjectURL(file.preview);
          pending.current.delete(file.key);
        }
        refresh();
      }}
    />
  );
}
