import { useState, type KeyboardEvent } from "react";
import { Send, Square } from "lucide-react";
import { Button } from "@/components/ui/button";

export function ChatComposer({
  active,
  sending,
  onSend,
  onStop,
}: {
  readonly active: boolean;
  readonly sending: boolean;
  readonly onSend: (content: string) => Promise<void>;
  readonly onStop: () => Promise<void>;
}) {
  const [content, setContent] = useState("");

  async function submit() {
    const message = content.trim();
    if (!message || active || sending) return;
    setContent("");
    try {
      await onSend(message);
    } catch {
      setContent(message);
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void submit();
    }
  }

  return (
    <div
      className="mx-auto w-full max-w-[728px] shrink-0 px-4 pt-2 sm:px-6"
      style={{ paddingBottom: "max(1rem, var(--sab))" }}
    >
      <div className="rounded-xl border border-hairline bg-card p-2 transition-colors focus-within:border-hairline-strong">
        <textarea
          value={content}
          onChange={(event) => setContent(event.target.value)}
          onKeyDown={handleKeyDown}
          disabled={active}
          rows={2}
          maxLength={32_768}
          placeholder={
            active ? "Assistant is working..." : "Message NyxID Assistant..."
          }
          className="max-h-40 min-h-[42px] w-full resize-none bg-transparent px-2 py-1 text-[13px] leading-relaxed text-foreground outline-none placeholder:text-text-tertiary disabled:cursor-not-allowed disabled:opacity-50"
        />
        <div className="flex items-center justify-end">
          {active ? (
            <Button
              type="button"
              variant="outline"
              size="icon"
              onClick={() => void onStop()}
              aria-label="Stop assistant turn"
            >
              <Square className="fill-current" />
            </Button>
          ) : (
            <Button
              type="button"
              variant="primary"
              size="icon"
              disabled={!content.trim() || sending}
              onClick={() => void submit()}
              aria-label="Send message"
            >
              <Send />
            </Button>
          )}
        </div>
      </div>
      <p className="pt-2 text-center text-[10px] text-text-tertiary">
        Every agent action is brokered, scoped, and audit-logged by NyxID.
      </p>
    </div>
  );
}
