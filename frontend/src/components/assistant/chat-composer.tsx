import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
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

/**
 * Whether the composer may take focus without being asked to.
 *
 * On a touch device, focusing a textarea raises the on-screen keyboard over
 * half the screen. Opening a conversation to read it is the common case there,
 * so autofocus would cost a dismiss on nearly every visit; a pointer is the
 * closest available proxy for "typing is cheap here".
 */
function autoFocusAllowed(): boolean {
  if (typeof window === "undefined") return false;
  // No matchMedia (jsdom, older engines) means no evidence of a touch device.
  if (typeof window.matchMedia !== "function") return true;
  return !window.matchMedia("(pointer: coarse)").matches;
}

/**
 * Whether focus is sitting on nothing in particular.
 *
 * Only the turn-end restore consults this. A turn can end while the reader is
 * mid-way through an approval card or a sidebar row, and pulling focus out
 * from under that is worse than the click it saves.
 */
function focusIsParked(textarea: HTMLTextAreaElement): boolean {
  const active = textarea.ownerDocument.activeElement;
  return (
    active === null ||
    active === textarea.ownerDocument.body ||
    active === textarea
  );
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
  /** Keep the composer writable while a typed actor task accepts steering. */
  readonly allowActiveInput?: boolean;
  readonly sending: boolean;
  readonly disabled?: boolean;
  readonly stopDisabled?: boolean;
  readonly ownerUserId: string | null;
  readonly draftKey: string | null;
  /**
   * Bumped by the page when the reader explicitly asks to be put in a chat —
   * a sidebar row or New Chat. Every change takes focus. Deliberately NOT
   * `draftKey`: the URL also moves on its own (canonical-id repair, stale-route
   * cleanup, the first send's migration off a draft thread) and none of those
   * are a request to start typing.
   */
  readonly focusRequest?: number;
  readonly controls?: ReactNode;
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
  allowActiveInput = false,
  sending,
  disabled = false,
  stopDisabled = false,
  ownerUserId,
  draftKey,
  focusRequest = 0,
  controls,
  onSend,
  onStop,
}: ChatComposerProps) {
  const locked = active && !allowActiveInput;
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

  /**
   * A turn disables the textarea, and the browser answers that by dropping
   * focus to the body — so focus has to be given back when the turn ends or
   * the reader has to click into the composer after every single answer.
   *
   * Restore only what the turn took, though. Whether the composer held focus
   * has to be read HERE, during the render that flips `active`, because by the
   * time an effect runs the commit has already disabled the field and the
   * browser has already moved focus away.
   *
   * The `disabled` guard is what keeps that read honest. A render that turns
   * out to be abandoned can leave `renderedActiveRef` reporting a transition
   * that never committed, and a later render would then re-read focus against
   * a DOM whose field is already disabled — overwriting a true flag with the
   * body. If the committed field is disabled, the turn is already under way
   * and there is nothing new to learn.
   */
  const composerHeldFocusRef = useRef(false);
  const renderedActiveRef = useRef(locked);
  if (renderedActiveRef.current !== locked) {
    if (locked && textareaRef.current?.disabled !== true) {
      composerHeldFocusRef.current = Boolean(
        composerRef.current?.contains(document.activeElement),
      );
    }
    renderedActiveRef.current = locked;
  }

  /**
   * The reader has moved on: since this turn started they have clicked,
   * typed, scrolled or tapped somewhere that is not the composer.
   *
   * DOM focus cannot answer that on its own. Scrolling the transcript,
   * selecting an answer to copy, and driving a screen reader's virtual cursor
   * all leave `activeElement` on the body — the exact state a disabled
   * textarea leaves behind — so "focus is parked" reads identically whether
   * the turn took it or the reader put it down and went elsewhere.
   *
   * Listens from the moment the send starts, not from `active`. Those are not
   * the same instant: the field is disabled by `active` alone, and a first
   * send waits on conversation creation and cache projection (deadline 5s)
   * before the transport publishes `running`. Watching only `active` leaves
   * that whole interval unobserved, and a reader who scrolls away inside it
   * would still have focus pulled back at the end of the turn.
   *
   * The window is one boolean on purpose. Keying the effect on `active` and
   * `sending` separately would tear it down and re-arm — clearing the flag —
   * at the handover between them, wiping exactly the movement it exists to
   * catch.
   */
  const readerMovedOnRef = useRef(false);
  const turnRunning = active || sending;
  useEffect(() => {
    if (!turnRunning) return;
    readerMovedOnRef.current = false;
    function noteInteraction(event: Event) {
      const target = event.target;
      if (target instanceof Node && composerRef.current?.contains(target)) {
        return;
      }
      readerMovedOnRef.current = true;
    }
    const types = ["pointerdown", "keydown", "wheel", "touchstart"] as const;
    for (const type of types) {
      document.addEventListener(type, noteInteraction, true);
    }
    return () => {
      for (const type of types) {
        document.removeEventListener(type, noteInteraction, true);
      }
    };
  }, [turnRunning]);

  /**
   * The composer is the only thing anyone opens this screen to use, so it
   * holds focus by default: on arrival, on every explicit `focusRequest`, and
   * when a turn hands the field back.
   *
   * An explicit request takes focus outright — including off the sidebar row
   * that was just clicked, which IS the request. Everything else is
   * conditional: a turn ending restores only focus the turn itself took and
   * only if focus is still parked, and a `draftKey` that moves without a
   * request (browser Back, canonical-id repair) claims only parked focus.
   *
   * Parked is a weaker signal than it looks — a reader scrolling the
   * transcript also leaves focus on the body — but outside a running turn
   * there is nothing better to go on, and the automatic `draftKey` moves are
   * canonical-id repair and stale-route cleanup, both of which land within
   * moments of the conversation opening rather than at an arbitrary time.
   *
   * A request that lands while the field is disabled is held rather than
   * dropped — but it is dropped if the reader moves on in the meantime, since
   * by then it is a minutes-old intent. See `autoFocusAllowed` for why touch
   * devices opt out.
   */
  const focusPendingRef = useRef(true);
  const seenFocusRequestRef = useRef(focusRequest);
  const previouslyActiveRef = useRef(locked);
  const previousDraftKeyRef = useRef(draftKey);
  useEffect(() => {
    const element = textareaRef.current;
    if (!element) return;

    const requested = seenFocusRequestRef.current !== focusRequest;
    const draftKeyMoved = previousDraftKeyRef.current !== draftKey;
    const turnJustEnded = previouslyActiveRef.current && !locked;
    seenFocusRequestRef.current = focusRequest;
    previousDraftKeyRef.current = draftKey;
    previouslyActiveRef.current = locked;

    if (requested) {
      // They just asked for this composer; nothing before it still counts.
      readerMovedOnRef.current = false;
      focusPendingRef.current = true;
    } else if (
      (draftKeyMoved || (turnJustEnded && composerHeldFocusRef.current)) &&
      focusIsParked(element)
    ) {
      focusPendingRef.current = true;
    }

    // The flag is a verdict on the turn that set it, so it is spent the moment
    // the field comes back — whether or not anything is waiting to consume it.
    // Leaving it set past its turn would let it veto an unrelated focus much
    // later: a reader who stepped away mid-turn, came back, and pressed Back
    // would land in a chat with no caret in it.
    const readerMovedOn = readerMovedOnRef.current;
    if (!element.disabled) readerMovedOnRef.current = false;

    if (!focusPendingRef.current) return;
    // Still disabled — a turn is running. Keep the request and honour it when
    // the field comes back rather than firing into a control that cannot take
    // focus at all.
    if (element.disabled) return;
    focusPendingRef.current = false;
    if (readerMovedOn || !autoFocusAllowed()) return;
    element.focus();
    // Restored drafts arrive whole; the caret belongs after them, not at the
    // start of what the reader was last writing.
    const caret = element.value.length;
    element.setSelectionRange(caret, caret);
  }, [draftKey, focusRequest, locked]);

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
    if (!message || locked || disabled || sending) return;
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
        {controls}
        <div
          ref={composerRef}
          className={`relative ml-[30px] flex gap-1.5 rounded-xl border border-hairline bg-card px-3 py-2 transition-colors focus-within:border-hairline-strong ${
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
              disabled={locked || disabled}
              rows={1}
              maxLength={32_768}
              placeholder={
                locked
                  ? "Assistant is working..."
                  : disabled
                    ? "This conversation is read-only."
                    : allowActiveInput
                     ? "Steer active task..."
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
            {locked ? (
              <Button
                type="button"
                variant="outline"
                size="icon"
                onClick={() => void onStop()}
                disabled={stopDisabled}
                aria-label="Stop assistant turn"
              >
                <Square className="fill-current" />
              </Button>
            ) : (
              <Button
                type="button"
                variant="primary"
                size="icon"
                disabled={!content.trim() || sending || disabled}
                onClick={() => void submit()}
                aria-label={
                  allowActiveInput
                    ? "Send steering instruction"
                    : "Send message"
                }
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
