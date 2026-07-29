import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { Send, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";

const DRAFT_DEBOUNCE_MS = 300;

function readOwnedDraft(userId: string | null, key: string | null): string {
  if (!userId || !key) return "";
  const store = useAssistantDraftStore.getState();
  return store.ownerUserId === userId ? store.getDraft(key) : "";
}

/**
 * The composer floats over the thread, so it cannot be allowed to eat the
 * conversation. It starts as a single line and grows with the draft up to this
 * many rows, after which the textarea scrolls internally.
 */
const MAX_ROWS = 4;

export function ChatComposer({
  ownerUserId,
  draftKey,
  ...props
}: ChatComposerProps) {
  return (
    <DraftedChatComposer
      {...props}
      ownerUserId={ownerUserId}
      draftKey={draftKey}
    />
  );
}

interface ChatComposerProps {
  readonly active: boolean;
  readonly sending: boolean;
  readonly ownerUserId: string | null;
  readonly draftKey: string | null;
  readonly onSend: (content: string) => Promise<void>;
  readonly onStop: () => Promise<void>;
}

interface PendingDraftTransition {
  readonly previousUserId: string | null;
  readonly previousKey: string | null;
  readonly nextUserId: string | null;
  readonly nextKey: string | null;
  readonly liveContent: string;
  readonly nextContent: string;
  readonly migrateScreenDraft: boolean;
}

function DraftedChatComposer({
  active,
  sending,
  ownerUserId,
  draftKey,
  onSend,
  onStop,
}: ChatComposerProps) {
  const [content, setContent] = useState(() =>
    readOwnedDraft(ownerUserId, draftKey),
  );
  const contentRef = useRef(content);
  const ownerUserIdRef = useRef(ownerUserId);
  const draftKeyRef = useRef(draftKey);
  const renderedOwnerUserIdRef = useRef(ownerUserId);
  const renderedDraftKeyRef = useRef(draftKey);
  const draftTimerRef = useRef<number | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const composingRef = useRef(false);
  const pendingTransitionRef = useRef<PendingDraftTransition | null>(null);

  if (
    renderedOwnerUserIdRef.current !== ownerUserId ||
    renderedDraftKeyRef.current !== draftKey
  ) {
    const previousUserId = ownerUserIdRef.current;
    const previousKey = draftKeyRef.current;
    const liveContent = contentRef.current;
    const store = useAssistantDraftStore.getState();
    const sameOwner = Boolean(
      ownerUserId &&
      previousUserId === ownerUserId &&
      store.ownerUserId === ownerUserId,
    );
    const incomingDraft = readOwnedDraft(ownerUserId, draftKey);
    const migrateScreenDraft = Boolean(
      sameOwner &&
      previousKey?.startsWith("screen:") &&
      draftKey?.startsWith("conv:") &&
      !incomingDraft,
    );
    const nextContent = migrateScreenDraft ? liveContent : incomingDraft;

    pendingTransitionRef.current = {
      previousUserId,
      previousKey,
      nextUserId: ownerUserId,
      nextKey: draftKey,
      liveContent,
      nextContent,
      migrateScreenDraft,
    };
    renderedOwnerUserIdRef.current = ownerUserId;
    renderedDraftKeyRef.current = draftKey;
    setContent(nextContent);
  }

  const cancelScheduledSave = useCallback(() => {
    if (draftTimerRef.current !== null) {
      window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = null;
    }
  }, []);

  const flushDraft = useCallback(() => {
    cancelScheduledSave();
    const userId = ownerUserIdRef.current;
    const key = draftKeyRef.current;
    const store = useAssistantDraftStore.getState();
    if (userId && key && store.ownerUserId === userId) {
      store.saveDraft(userId, key, contentRef.current);
    }
  }, [cancelScheduledSave]);

  const scheduleDraftSave = useCallback(() => {
    cancelScheduledSave();
    if (!ownerUserIdRef.current || !draftKeyRef.current) return;
    draftTimerRef.current = window.setTimeout(flushDraft, DRAFT_DEBOUNCE_MS);
  }, [cancelScheduledSave, flushDraft]);

  useEffect(() => {
    cancelScheduledSave();
    const transition = pendingTransitionRef.current;
    pendingTransitionRef.current = null;
    const store = useAssistantDraftStore.getState();
    if (transition) {
      const sameOwner = Boolean(
        transition.nextUserId &&
        transition.previousUserId === transition.nextUserId &&
        store.ownerUserId === transition.nextUserId,
      );
      if (sameOwner && transition.previousKey && transition.nextUserId) {
        store.saveDraft(
          transition.nextUserId,
          transition.previousKey,
          transition.liveContent,
        );
        if (transition.migrateScreenDraft && transition.nextKey) {
          store.clearDraft(transition.nextUserId, transition.previousKey);
          store.saveDraft(
            transition.nextUserId,
            transition.nextKey,
            transition.liveContent,
          );
        }
      }
      ownerUserIdRef.current = transition.nextUserId;
      draftKeyRef.current = transition.nextKey;
      contentRef.current = transition.nextContent;
    }
    if (ownerUserId && draftKey && store.ownerUserId !== ownerUserId) {
      store.saveDraft(ownerUserId, draftKey, "");
    }
  }, [cancelScheduledSave, draftKey, ownerUserId]);

  useEffect(() => {
    function handleBeforeUnload() {
      flushDraft();
    }

    window.addEventListener("beforeunload", handleBeforeUnload);
    return () => {
      window.removeEventListener("beforeunload", handleBeforeUnload);
      flushDraft();
    };
  }, [flushDraft]);

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
    cancelScheduledSave();
    const userId = ownerUserIdRef.current;
    const key = draftKeyRef.current;
    const store = useAssistantDraftStore.getState();
    if (userId && key && store.ownerUserId === userId) {
      store.clearDraft(userId, key);
    }
    contentRef.current = "";
    setContent("");
    try {
      await onSend(message);
    } catch {
      contentRef.current = message;
      setContent(message);
      scheduleDraftSave();
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
          onChange={(event) => {
            contentRef.current = event.target.value;
            setContent(event.target.value);
            if (!composingRef.current) scheduleDraftSave();
          }}
          onKeyDown={handleKeyDown}
          onCompositionStart={() => {
            composingRef.current = true;
            cancelScheduledSave();
          }}
          onCompositionEnd={(event) => {
            composingRef.current = false;
            contentRef.current = event.currentTarget.value;
            scheduleDraftSave();
          }}
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
