import {
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { Send, Square } from "lucide-react";
import { Button } from "@/components/ui/button";

/**
 * The composer floats over the thread, so it cannot be allowed to eat the
 * conversation. It starts as a single line and grows with the draft up to this
 * many rows, after which the textarea scrolls internally.
 */
const MAX_ROWS = 4;

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
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const composingRef = useRef(false);

  // Grow the textarea to fit the draft, capped at MAX_ROWS.
  useLayoutEffect(() => {
    const element = textareaRef.current;
    if (!element) return;

    const styles = getComputedStyle(element);
    const lineHeight = Number.parseFloat(styles.lineHeight) || 21;
    const padding =
      Number.parseFloat(styles.paddingTop) +
      Number.parseFloat(styles.paddingBottom);
    const oneRow = lineHeight + padding;
    const maxHeight = lineHeight * MAX_ROWS + padding;

    // Measure unconstrained, then put the old height back and force a reflow
    // before writing the target. Without that restore the browser's
    // before-change style is already the natural height, so the CSS height
    // transition has nothing to animate from and the box snaps.
    const previous = element.style.height;
    element.style.height = "auto";
    const natural = element.scrollHeight;
    element.style.height = previous;
    void element.offsetHeight;

    element.style.height = `${String(Math.max(Math.min(natural, maxHeight), oneRow))}px`;
    element.style.overflowY = natural > maxHeight ? "auto" : "hidden";
  }, [content]);

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
    const isComposing =
      composingRef.current ||
      event.nativeEvent.isComposing ||
      event.keyCode === 229;
    if (event.key === "Enter" && !event.shiftKey && !isComposing) {
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
          ref={textareaRef}
          value={content}
          onChange={(event) => setContent(event.target.value)}
          onCompositionStart={() => {
            composingRef.current = true;
          }}
          onCompositionEnd={() => {
            composingRef.current = false;
          }}
          onKeyDown={handleKeyDown}
          disabled={active}
          rows={1}
          maxLength={32_768}
          placeholder={
            active ? "Assistant is working..." : "Message NyxID Assistant..."
          }
          className="w-full resize-none overflow-hidden bg-transparent px-2 py-1 text-[13px] leading-relaxed text-foreground outline-none transition-[height] duration-150 ease-out placeholder:text-text-tertiary disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none"
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
