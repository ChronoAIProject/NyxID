import { useEffect, useRef, useState } from "react";
import { Check, Copy, LoaderCircle } from "lucide-react";

/** Copies only the renderer's body value, never DOM controls or hidden blocks. */
export function MessageCopyAction({ text }: { readonly text: string }) {
  const [result, setResult] = useState<{ text: string; state: "idle" | "copying" | "copied" | "failed" }>({ text, state: "idle" });
  const state = result.text === text ? result.state : "idle";
  const setState = (state: typeof result.state) => setResult({ text, state });
  const attempt = useRef(0);
  useEffect(() => {
    attempt.current += 1;
    return () => { attempt.current += 1; };
  }, [text]);
  if (!text.trim()) return null;
  const label = state === "copied" ? "Copied" : state === "copying" ? "Copying…" : "Copy message";
  const Icon = state === "copied" ? Check : state === "copying" ? LoaderCircle : Copy;
  return (
    <span className="inline-flex flex-wrap items-center gap-1 text-11">
      <button type="button" aria-label={label} title={label} disabled={state === "copying"}
        className="inline-flex min-h-8 min-w-8 items-center justify-center gap-1 rounded px-2 text-text-tertiary hover:bg-overlay focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring disabled:opacity-50"
        onClick={() => {
          const current = ++attempt.current;
          setState("copying");
          void (async () => {
            try {
              await navigator.clipboard.writeText(text);
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
