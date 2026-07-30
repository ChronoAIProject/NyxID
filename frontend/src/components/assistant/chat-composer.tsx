import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type UIEvent,
} from "react";
import { Send, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";

const DRAFT_DEBOUNCE_MS = 300;
const MAX_ROWS = 4;
const MULTILINE_THRESHOLD = 0.95;

type ScrollEdges = {
  readonly top: boolean;
  readonly bottom: boolean;
};

function readOwnedDraft(userId: string | null, key: string | null): string {
  if (!userId || !key) return "";
  const store = useAssistantDraftStore.getState();
  return store.ownerUserId === userId ? store.getDraft(key) : "";
}

function getScrollEdges(element: HTMLTextAreaElement): ScrollEdges {
  return {
    top: element.scrollTop > 1,
    bottom: element.scrollTop + element.clientHeight < element.scrollHeight - 1,
  };
}

function cssPixels(value: string): number {
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

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
  const [multiline, setMultiline] = useState(false);
  const [scrollEdges, setScrollEdges] = useState<ScrollEdges>({
    top: false,
    bottom: false,
  });
  const contentRef = useRef(content);
  const ownerUserIdRef = useRef(ownerUserId);
  const draftKeyRef = useRef(draftKey);
  const renderedOwnerUserIdRef = useRef(ownerUserId);
  const renderedDraftKeyRef = useRef(draftKey);
  const draftTimerRef = useRef<number | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const composerRef = useRef<HTMLDivElement>(null);
  const controlsRef = useRef<HTMLDivElement>(null);
  const textMeasureRef = useRef<HTMLSpanElement>(null);
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

  const syncComposerLayout = useCallback((draft?: string) => {
    const composer = composerRef.current;
    const controls = controlsRef.current;
    const textMeasure = textMeasureRef.current;
    if (!composer || !controls || !textMeasure) return;

    const measuredDraft = draft ?? textMeasure.textContent ?? "";
    const styles = getComputedStyle(composer);
    const contentWidth =
      composer.clientWidth -
      cssPixels(styles.paddingLeft) -
      cssPixels(styles.paddingRight);
    const availableInlineWidth =
      contentWidth -
      controls.getBoundingClientRect().width -
      cssPixels(styles.columnGap);
    const textWidth = textMeasure.getBoundingClientRect().width;
    const shouldUseMultiline =
      measuredDraft.includes("\n") ||
      (availableInlineWidth > 0 &&
        textWidth >= availableInlineWidth * MULTILINE_THRESHOLD);

    setMultiline((current) =>
      current === shouldUseMultiline ? current : shouldUseMultiline,
    );
  }, []);

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

  useLayoutEffect(() => {
    const composer = composerRef.current;
    const controls = controlsRef.current;
    const textMeasure = textMeasureRef.current;
    if (
      !composer ||
      !controls ||
      !textMeasure ||
      typeof ResizeObserver === "undefined"
    ) {
      return;
    }

    const observer = new ResizeObserver(() => syncComposerLayout());
    observer.observe(composer);
    observer.observe(controls);
    observer.observe(textMeasure);
    return () => observer.disconnect();
  }, [syncComposerLayout]);

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

    // Restore the old height before writing the target so the transition has
    // a stable before-change value instead of snapping to the natural height.
    const previous = element.style.height;
    element.style.height = "auto";
    const natural = element.scrollHeight;
    element.style.height = previous;
    void element.offsetHeight;

    element.style.height = `${String(Math.max(Math.min(natural, maxHeight), oneRow))}px`;
    element.style.overflowY = natural > maxHeight ? "auto" : "hidden";
    setScrollEdges(getScrollEdges(element));
  }, [content]);

  function updateContent(nextContent: string) {
    contentRef.current = nextContent;
    if (textMeasureRef.current) {
      textMeasureRef.current.textContent = nextContent;
    }
    syncComposerLayout(nextContent);
    setContent(nextContent);
  }

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
    updateContent("");
    try {
      await onSend(message);
    } catch {
      updateContent(message);
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

  function handleScroll(event: UIEvent<HTMLTextAreaElement>) {
    setScrollEdges(getScrollEdges(event.currentTarget));
  }

  return (
    <div
      className="shrink-0"
      style={{
        width: "calc(100% - var(--assistant-scrollbar-width, 0px))",
      }}
    >
      <div
        className="mx-auto w-full max-w-[758px] px-4 pt-2 sm:px-6"
        style={{ paddingBottom: "max(1rem, var(--sab))" }}
      >
        <div
          ref={composerRef}
          className={`relative ml-[30px] flex gap-1.5 rounded-xl border border-hairline bg-card p-1.5 transition-colors focus-within:border-hairline-strong ${
            multiline ? "flex-col items-stretch" : "items-start"
          }`}
        >
          <span
            ref={textMeasureRef}
            aria-hidden
            className="pointer-events-none absolute invisible inline-block w-max whitespace-pre text-[13px] leading-relaxed"
          >
            {content}
          </span>
          <div className="relative min-w-0 flex-1">
            <textarea
              ref={textareaRef}
              value={content}
              onChange={(event) => {
                updateContent(event.target.value);
                if (!composingRef.current) scheduleDraftSave();
              }}
              onKeyDown={handleKeyDown}
              onCompositionStart={() => {
                composingRef.current = true;
                cancelScheduledSave();
              }}
              onCompositionEnd={(event) => {
                composingRef.current = false;
                updateContent(event.currentTarget.value);
                scheduleDraftSave();
              }}
              onScroll={handleScroll}
              disabled={active}
              rows={1}
              maxLength={32_768}
              placeholder={
                active
                  ? "Assistant is working..."
                  : "Message NyxID Assistant..."
              }
              className="assistant-scrollbar block min-h-8 w-full resize-none overflow-hidden bg-transparent px-0 py-1 text-[13px] leading-relaxed text-foreground outline-none transition-[height] duration-150 ease-out placeholder:text-text-tertiary disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none"
            />
            <div
              aria-hidden
              className={`pointer-events-none absolute inset-x-0 top-0 h-3 bg-gradient-to-b from-card to-transparent transition-opacity duration-150 motion-reduce:transition-none ${
                scrollEdges.top ? "opacity-100" : "opacity-0"
              }`}
            />
            <div
              aria-hidden
              className={`pointer-events-none absolute inset-x-0 bottom-0 h-3 bg-gradient-to-t from-card to-transparent transition-opacity duration-150 motion-reduce:transition-none ${
                scrollEdges.bottom ? "opacity-100" : "opacity-0"
              }`}
            />
          </div>
          <div
            ref={controlsRef}
            className={`flex shrink-0 items-center ${multiline ? "self-end" : ""}`}
          >
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
      </div>
    </div>
  );
}
