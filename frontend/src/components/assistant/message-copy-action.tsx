import { useEffect, useRef, useState } from "react";
import { Check, Copy, LoaderCircle } from "lucide-react";
import { copyMessage, type MessageCopyFormat } from "@/lib/assistant/message-clipboard";
import type { InlineMention } from "@/lib/assistant/group-mentions";

/** Copies only the renderer's body value, never DOM controls or hidden blocks. */
export function MessageCopyAction({ text, format = "plain", mentions }: {
  readonly text: string;
  readonly format?: MessageCopyFormat;
  readonly mentions?: readonly InlineMention[];
}) {
  if (!text.trim()) return null;
  // A body/format change owns a fresh attempt, even A → B → A. Old promises
  // cannot update the replacement control or resurrect an earlier success.
  return <CopyButton key={JSON.stringify([format, text, mentions])} text={text} format={format} mentions={mentions} />;
}

function CopyButton({ text, format, mentions }: { readonly text: string; readonly format: MessageCopyFormat; readonly mentions?: readonly InlineMention[] }) {
  const [state, setState] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  const attempt = useRef(0);
  useEffect(() => {
    attempt.current += 1;
    return () => { attempt.current += 1; };
  }, []);
  const label = state === "copied" ? "Copied" : state === "copying" ? "Copying…" : "Copy message";
  const Icon = state === "copied" ? Check : state === "copying" ? LoaderCircle : Copy;
  return (
    <span className="inline-flex flex-wrap items-center gap-1 text-13">
      <button type="button" aria-label={label} title={label} disabled={state === "copying"} data-icon-button={state === "copied" ? undefined : ""}
        className="inline-flex min-h-8 min-w-8 items-center justify-center gap-1 rounded px-2 text-text-tertiary hover:bg-overlay focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-50"
        onClick={() => {
          const current = ++attempt.current;
          setState("copying");
          void (async () => {
            try {
              await copyMessage(text, format, mentions);
              if (attempt.current === current) setState("copied");
            } catch {
              if (attempt.current === current) setState("failed");
            }
          })();
        }}>
        <Icon aria-hidden="true" className="size-3.5" />
        {state === "copied" ? <span>Copied</span> : null}
      </button>
      {state === "failed" || state === "copied" ? <span role="status" aria-live="polite" className={state === "failed" ? "text-destructive" : "sr-only"}>
        {state === "failed" ? "Copy failed. Try Copy message again." : state === "copied" ? "Message copied." : ""}
      </span> : null}
    </span>
  );
}
