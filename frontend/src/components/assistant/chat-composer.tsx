import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
  type PointerEvent,
} from "react";
import { ArrowUp, AudioLines, FileText, ImageIcon, Square } from "lucide-react";
import { ComposerEditor, type ComposerEditorHandle, type ComposerSelection } from "./composer-editor";
import { tokenSegments, type ComposerToken } from "./composer-editor-model";
import { Button } from "@/components/ui/button";
import { AgentAvatar } from "@/components/assistant/nyxbot-agent-avatar";
import {
  activeMention,
  insertMention,
  mentionCandidates,
  type MentionQuery,
} from "@/lib/assistant/nyxbot-mentions";
import {
  isLongPaste,
  stripAttachmentMention,
  type AttachmentMention,
} from "@/lib/assistant/attachment-mentions";
import { isAdmittedSendError } from "@/lib/assistant/send-admission";
import { useComposerDictation } from "./composer-dictation";
import type { AudioActivity } from "@/lib/microphone-capture";
import { cn } from "@/lib/utils";
import type { AssistantAgentKind } from "@/schemas/assistant-nyxagent";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { COMPOSER_MAX_HEIGHT, useThemeStore } from "@/stores/theme-store";

const DRAFT_DEBOUNCE_MS = 300;
const MIN_TEXT_HEIGHT: number = COMPOSER_MAX_HEIGHT.min;

function composerHeightLimit(
  surface: HTMLElement | null,
  editor: HTMLElement | undefined,
): number {
  const viewport = window.visualViewport;
  const viewportHeight = viewport?.height ?? window.innerHeight;
  const chromeHeight = surface && editor
    ? surface.offsetHeight - editor.offsetHeight
    : 0;
  // Keep the grip below the chat header, including on short/mobile viewports.
  const visibleTop = Math.max(
    viewport?.offsetTop ?? 0,
    surface?.closest("main")?.getBoundingClientRect().top ?? 0,
  );
  const bottom = surface?.getBoundingClientRect().bottom;
  const availableHeight = bottom ? bottom - visibleTop - 8 : viewportHeight;
  return Math.max(32, Math.floor(
    Math.min(viewportHeight * 0.9, availableHeight) - chromeHeight,
  ));
}
const PASTED_EXTENSIONS: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
};

/** Clipboard images arrive unnamed or with a generic `image.<ext>` name. */
function isGenericPasteName(name: string): boolean {
  return !name || /^image\.[a-z0-9]+$/i.test(name);
}

/** Keeps the clipboard's own format: a JPEG stays `.jpg`, a GIF `.gif`. */
function pastedExtension(type: string): string {
  return PASTED_EXTENSIONS[type] ?? (type.split("/")[1]?.split("+")[0]?.replace(/[^a-z0-9]/gi, "") || "png");
}

type ScrollEdges = {
  readonly top: boolean;
  readonly bottom: boolean;
  readonly thumb: { readonly size: number; readonly top: number } | null;
};

function readOwnedDraft(userId: string | null, key: string | null): string {
  if (!userId || !key) return "";
  const store = useAssistantDraftStore.getState();
  return store.ownerUserId === userId ? store.getDraft(key) : "";
}

function getScrollEdges(element: HTMLElement): ScrollEdges {
  const range = element.scrollHeight - element.clientHeight;
  const size = Math.min(100, Math.max(12, element.clientHeight / element.scrollHeight * 100));
  return {
    thumb: range > 1 ? { size, top: Math.max(0, Math.min(1, element.scrollTop / range)) * (100 - size) } : null,
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
function focusIsParked(textarea: HTMLElement): boolean {
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

type MentionOption =
  | { readonly type: "agent"; readonly id: string; readonly name: string; readonly agent: ComposerMention }
  | {
      readonly type: "attachment";
      readonly id: string;
      readonly name: string;
      readonly attachment: AttachmentMention;
    };

/** An agent that can be `@mentioned` from the composer (group chats). */
export interface ComposerMention {
  readonly id: string;
  /** The @handle that is inserted. */
  readonly name: string;
  readonly kind: AssistantAgentKind;
  readonly display_name?: string | null;
}

interface ChatComposerProps {
  readonly active: boolean;
  /** Keep the composer writable while a typed actor task accepts steering. */
  readonly allowActiveInput?: boolean;
  readonly sending: boolean;
  readonly activePlaceholder?: string;
  readonly sendLabel?: string;
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
  /** Settings beside the action button; move to the footer when text expands. */
  readonly footerControls?: ReactNode;
  readonly attachmentButton?: ReactNode;
  readonly attachments?: ReactNode;
  readonly recipients?: ReactNode;
  readonly hasAttachments?: boolean;
  readonly uploadBlocked?: boolean;
  readonly onFiles?: (files: File[]) => void;
  /** Idle placeholder, e.g. "Message NyxBot". */
  readonly placeholder?: string;
  /**
   * Group chats: typing `@` lists these agents; Enter or Tab inserts
   * `@name `, arrows move, Escape dismisses.
   */
  readonly mentions?: readonly ComposerMention[];
  /** Attached files, mentionable as `@handle` in the same list. */
  readonly attachmentMentions?: readonly AttachmentMention[];
  /**
   * A long paste: attach it as a file and return its handle, which is placed
   * at the caret. Undefined keeps it as ordinary pasted text.
   */
  readonly onLongText?: (text: string) => string | undefined;
  /** The text a saved draft keeps, when it differs from what is on screen. */
  readonly draftText?: (content: string) => string;
  readonly onSend: (content: string) => Promise<void>;
  readonly onContentChange?: (content: string) => void;
  readonly onStop: () => Promise<void>;
  readonly onVoice?: (configureOnly?: boolean) => void;
  readonly getDictationThread?: () => Promise<string>;
  readonly dictationThreadId?: string;
  readonly voiceControls?: React.ReactNode;
  readonly voiceActive?: boolean;
  readonly voiceActivity?: AudioActivity;
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
  activePlaceholder,
  sendLabel,
  disabled = false,
  stopDisabled = false,
  ownerUserId,
  draftKey,
  focusRequest = 0,
  controls,
  footerControls,
  attachmentButton,
  attachments,
  recipients,
  hasAttachments = false,
  uploadBlocked = false,
  onFiles,
  placeholder,
  mentions,
  attachmentMentions,
  onLongText,
  draftText,
  onSend,
  onContentChange,
  onStop,
  onVoice,
  getDictationThread,
  dictationThreadId,
  voiceControls,
  voiceActive = false,
  voiceActivity,
}: ChatComposerProps) {
  const tokens: ComposerToken[] = [
    ...(mentions ?? []).map((agent) => ({ id: agent.id, handle: agent.name, label: agent.display_name?.trim() || agent.name, kind: "agent" as const })),
    ...(attachmentMentions ?? []).map((file) => ({ id: `attachment:${file.handle}`, handle: file.handle, label: file.handle, kind: file.kind })),
  ];
  const locked = active && !allowActiveInput;
  const [dragging, setDragging] = useState(false);
  const dragDepth = useRef(0);
  // Unique across pastes so each chip's Remove/Preview label names one file.
  const pastedImages = useRef(0);
  const mentionListId = `${useId()}-mentions`;
  const [mention, setMention] = useState<MentionQuery>();
  const [mentionIndex, setMentionIndex] = useState(0);
  const mentionOptions: MentionOption[] = mention
    ? [
        ...mentionCandidates(mentions ?? [], mention.query).map((agent) => ({
          type: "agent" as const,
          id: agent.id,
          name: agent.name,
          agent,
        })),
        ...mentionCandidates(
          (attachmentMentions ?? []).map((attachment) => ({
            ...attachment,
            name: attachment.handle,
            display_name: attachment.label,
          })),
          mention.query,
        ).map((attachment) => ({
          type: "attachment" as const,
          id: `attachment:${attachment.handle}`,
          name: attachment.handle,
          attachment,
        })),
      ]
    : [];
  const mentionOpen = mentionOptions.length > 0;
  const selectedMention = mentionOpen
    ? mentionOptions[Math.min(mentionIndex, mentionOptions.length - 1)]
    : undefined;
  const [content, setContent] = useState(() =>
    readOwnedDraft(ownerUserId, draftKey),
  );
  const [multiline, setMultiline] = useState(false);
  const savedMaxHeight = useThemeStore((state) => state.composerMaxHeight);
  const setMaxTextHeight = useThemeStore((state) => state.setComposerMaxHeight);
  const [previewHeight, setPreviewHeight] = useState<number | null>(null);
  const maxTextHeight = previewHeight ?? savedMaxHeight;
  const [resizedTextHeight, setResizedTextHeight] = useState<number>();
  const [resizing, setResizing] = useState(false);
  const [heightRange, setHeightRange] = useState({ current: 92, min: MIN_TEXT_HEIGHT, max: 480 });
  const resizeStartRef = useRef<{ pointer: number; y: number; height: number; current: number } | null>(null);
  function cancelHeightResize() {
    resizeStartRef.current = null;
    setPreviewHeight(null);
    setResizing(false);
  }
  const expanded = multiline;
  const hasDraftContent = Boolean(
    content.trim() || hasAttachments || attachmentMentions?.length,
  );
  const showVoice = Boolean(
    onVoice && !hasDraftContent && (voiceActive || (!active && !uploadBlocked)),
  );
  const [scrollEdges, setScrollEdges] = useState<ScrollEdges>({
    top: false,
    bottom: false,
    thumb: null,
  });
  const contentRef = useRef(content);
  const ownerUserIdRef = useRef(ownerUserId);
  const draftKeyRef = useRef(draftKey);
  const renderedOwnerUserIdRef = useRef(ownerUserId);
  const renderedDraftKeyRef = useRef(draftKey);
  const draftTimerRef = useRef<number | null>(null);
  const editorRef = useRef<ComposerEditorHandle>(null);
  const dictationSelection = useRef<ComposerSelection | null>(null);
  const reviewFocus = useRef<{ start: number; end: number } | null>(null);
  const dictation = useComposerDictation({
    scope: `${ownerUserId}:${draftKey}`,
    enabled: Boolean(getDictationThread),
    disabled: disabled || locked || sending || voiceActive,
    getThread: getDictationThread,
    configureVoice: () => {
      dictation.releaseMicrophone();
      flushDraft();
      onVoice?.(true);
    },
    threadId: dictationThreadId,
    begin: () => {
      const editor = editorRef.current;
      if (editor)
        dictationSelection.current = {
          value: editor.value,
          selectionStart: editor.selectionStart,
          selectionEnd: editor.selectionEnd,
        };
    },
    insert: (text) => {
      const selection = dictationSelection.current;
      if (!selection || !editorRef.current?.insertDictation(text, selection))
        return false;
      dictationSelection.current = {
        ...selection,
        selectionStart: selection.selectionStart + text.length,
        selectionEnd: selection.selectionStart + text.length,
      };
      scheduleDraftSave();
      return true;
    },
    reviewClosed: () => {
      const selection = dictationSelection.current;
      if (selection)
        reviewFocus.current = {
          start: selection.selectionStart,
          end: selection.selectionEnd,
        };
    },
  });
  useLayoutEffect(() => {
    if (dictation.active || disabled || locked || !reviewFocus.current) return;
    const selection = reviewFocus.current;
    reviewFocus.current = null;
    editorRef.current?.focus();
    editorRef.current?.setSelectionRange(selection.start, selection.end);
  });
  const audioLevel = Math.max(
    dictation.activity.input,
    voiceActivity?.input ?? 0,
    voiceActivity?.output ?? 0,
  );
  /** Where the caret goes once an inserted mention is committed. */
  const pendingCaretRef = useRef<number | null>(null);
  useLayoutEffect(() => {
    const element = editorRef.current;
    const caret = pendingCaretRef.current;
    if (!element || caret === null) return;
    pendingCaretRef.current = null;
    element.focus();
    element.setSelectionRange(caret, caret);
  });
  const composerRef = useRef<HTMLDivElement>(null);
  const bandRef = useRef<HTMLDivElement>(null);
  const surfaceRef = useRef<HTMLDivElement>(null);
  const controlsRef = useRef<HTMLDivElement>(null);
  const settingsRef = useRef<HTMLDivElement>(null);
  const attachmentRef = useRef<HTMLDivElement>(null);
  const textMeasureRef = useRef<HTMLSpanElement>(null);
  const composingRef = useRef(false);
  const pendingTransitionRef = useRef<PendingDraftTransition | null>(null);
  const draftTextRef = useRef(draftText);
  draftTextRef.current = draftText;
  const savedText = (text: string) => draftTextRef.current?.(text) ?? text;

  if (
    renderedOwnerUserIdRef.current !== ownerUserId ||
    renderedDraftKeyRef.current !== draftKey
  ) {
    reviewFocus.current = null;
    dictationSelection.current = null;
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
    onContentChange?.(nextContent);
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
    const gap = cssPixels(styles.columnGap);
    const settingsWidth = settingsRef.current?.getBoundingClientRect().width ?? 0;
    // Expanded text gains the attachment slot's width. Measure against the
    // compact row in both layouts so crossing the threshold cannot oscillate.
    const attachmentWidth = composer.classList.contains("flex-col")
      ? (attachmentRef.current?.getBoundingClientRect().width ?? 0)
      : 0;
    const availableInlineWidth =
      contentWidth - controls.getBoundingClientRect().width - gap -
      (settingsWidth ? settingsWidth + gap : 0) -
      (attachmentWidth ? attachmentWidth + gap : 0);
    const textWidth = textMeasure.getBoundingClientRect().width;
    const shouldUseMultiline =
      measuredDraft.includes("\n") ||
      (availableInlineWidth > 0 &&
        textWidth > availableInlineWidth);

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
      store.saveDraft(userId, key, savedText(contentRef.current));
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
          savedText(transition.liveContent),
        );
        if (transition.migrateScreenDraft && transition.nextKey) {
          store.clearDraft(transition.nextUserId, transition.previousKey);
          store.saveDraft(
            transition.nextUserId,
            transition.nextKey,
            savedText(transition.liveContent),
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

    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => syncComposerLayout());
    });
    observer.observe(composer);
    observer.observe(controls);
    observer.observe(textMeasure);
    if (settingsRef.current) observer.observe(settingsRef.current);
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, [syncComposerLayout]);

  // Auto-size until the user resizes. A multiline draft then follows the chosen
  // visible height, including empty space; single-line drafts remain compact.
  const syncEditorHeight = useCallback(() => {
    const element = editorRef.current?.element;
    if (!element) return;

    const styles = getComputedStyle(element);
    const lineHeight = Number.parseFloat(styles.lineHeight) || 21;
    const padding =
      Number.parseFloat(styles.paddingTop) +
      Number.parseFloat(styles.paddingBottom);
    const oneRow = lineHeight + padding;
    const limit = Math.min(COMPOSER_MAX_HEIGHT.max, composerHeightLimit(surfaceRef.current, element));
    if (bandRef.current) {
      // Extra height overlays the transcript instead of growing its bottom inset.
      bandRef.current.style.maxHeight = `${Math.min(480, (window.visualViewport?.height ?? window.innerHeight) / 2)}px`;
    }
    // Restore the old height before writing the target so the transition has
    // a stable before-change value instead of snapping to the natural height.
    const previous = element.style.height;
    element.style.height = "auto";
    const natural = element.scrollHeight;
    element.style.height = previous;
    void element.offsetHeight;

    const minHeight = Math.min(MIN_TEXT_HEIGHT, Math.max(natural, oneRow), limit);
    const maxHeight = Math.max(oneRow, Math.min(maxTextHeight, limit));
    const height = (previewHeight !== null || resizedTextHeight === savedMaxHeight) && expanded
      ? Math.max(minHeight, maxHeight)
      : Math.max(Math.min(natural, maxHeight), oneRow);
    setHeightRange((current) => current.current === Math.round(height) && current.min === minHeight && current.max === limit
      ? current
      : { current: Math.round(height), min: minHeight, max: limit });
    element.style.height = `${String(height)}px`;
    element.style.overflowY = natural > height ? "scroll" : "hidden";
    setScrollEdges(getScrollEdges(element));
  }, [resizedTextHeight, expanded, maxTextHeight, previewHeight, savedMaxHeight]);

  useLayoutEffect(() => {
    syncComposerLayout(content);
    syncEditorHeight();
  }, [content, expanded, syncEditorHeight, syncComposerLayout, draftKey, ownerUserId]);

  useEffect(() => {
    window.addEventListener("resize", syncEditorHeight);
    window.visualViewport?.addEventListener("resize", syncEditorHeight);
    return () => {
      window.removeEventListener("resize", syncEditorHeight);
      window.visualViewport?.removeEventListener("resize", syncEditorHeight);
    };
  }, [syncEditorHeight]);

  useLayoutEffect(() => {
    const element = editorRef.current?.element;
    if (!element || typeof ResizeObserver === "undefined") return;
    let width = element.getBoundingClientRect().width;
    let chromeHeight = (surfaceRef.current?.offsetHeight ?? 0) - element.offsetHeight;
    let heightLimit = composerHeightLimit(surfaceRef.current, element);
    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const nextWidth = element.getBoundingClientRect().width;
        const nextChromeHeight = (surfaceRef.current?.offsetHeight ?? 0) - element.offsetHeight;
        const nextHeightLimit = composerHeightLimit(surfaceRef.current, element);
        if (nextWidth === width && nextChromeHeight === chromeHeight && nextHeightLimit === heightLimit) {
          setScrollEdges(getScrollEdges(element));
          return;
        }
        width = nextWidth;
        chromeHeight = nextChromeHeight;
        heightLimit = nextHeightLimit;
        syncEditorHeight();
      });
    });
    observer.observe(element);
    if (surfaceRef.current) observer.observe(surfaceRef.current);
    const chatSurface = surfaceRef.current?.closest("main");
    if (chatSurface) observer.observe(chatSurface);
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, [syncEditorHeight, draftKey, ownerUserId]);

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
    if (locked && editorRef.current?.disabled !== true) {
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
    const element = editorRef.current;
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
      focusIsParked(element.element)
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

  // A removed attachment takes its `@handle` references with it.
  const attachmentHandles = (attachmentMentions ?? []).map((item) => item.handle).join(" ");
  const previousHandlesRef = useRef(attachmentHandles);
  useEffect(() => {
    const previous = previousHandlesRef.current.split(" ").filter(Boolean);
    previousHandlesRef.current = attachmentHandles;
    const current = new Set(attachmentHandles.split(" "));
    const removed = previous.filter((handle) => !current.has(handle));
    if (!removed.length) return;
    const next = removed.reduce(stripAttachmentMention, contentRef.current);
    if (next === contentRef.current) return;
    updateContent(next);
    scheduleDraftSave();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [attachmentHandles]);

  function updateContent(nextContent: string) {
    contentRef.current = nextContent;
    syncComposerLayout(nextContent);
    setContent(nextContent);
    onContentChange?.(nextContent);
  }

  /** Re-read the mention under the caret after typing or moving the caret. */
  function syncMention(element: ComposerSelection) {
    if (!mentions?.length && !attachmentMentions?.length) return;
    const caret = element.selectionStart;
    const next = element.mentionQueryAllowed === false ? undefined :
      caret === element.selectionEnd ? activeMention(element.value, caret) : undefined;
    if (next?.start === mention?.start && next?.query === mention?.query) return;
    setMention(next);
    setMentionIndex(0);
  }

  function chooseMention(name: string) {
    const element = editorRef.current;
    if (!element || !mention) return;
    const next = insertMention(content, mention, element.selectionStart, name);
    // Placed right after React commits the new text (below), before any
    // further keystroke: a frame later, fast typing would land first and
    // the caret would jump back in front of it.
    pendingCaretRef.current = next.caret;
    updateContent(next.text);
    scheduleDraftSave();
    setMention(undefined);
  }

  /** Put `text` where the caret is, replacing any selection, as its own word. */
  function insertAtCaret(text: string) {
    const element = editorRef.current;
    const value = contentRef.current;
    const start = element?.selectionStart ?? value.length;
    const end = element?.selectionEnd ?? value.length;
    const before = value.slice(0, start);
    const spaced = before && !/\s$/.test(before) ? ` ${text}` : text;
    pendingCaretRef.current = start + spaced.length;
    updateContent(`${before}${spaced}${value.slice(end)}`);
    scheduleDraftSave();
  }

  async function submit() {
    const message = content.trim();
    if (
      (!message && !hasAttachments) ||
      uploadBlocked ||
      locked ||
      disabled ||
      sending ||
      dictation.active
    )
      return;
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
    } catch (error) {
      // An admitted message is already in the transcript; restoring it would
      // invite a duplicate send.
      if (isAdmittedSendError(error)) return;
      // Guidance can be sent while the field remains editable. Preserve any
      // newer draft typed while the request was in flight.
      if (!contentRef.current) updateContent(message);
      scheduleDraftSave();
    }
  }

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const isComposing =
      composingRef.current ||
      event.nativeEvent.isComposing ||
      event.keyCode === 229;
    if (mentionOpen && !isComposing) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        const step = event.key === "ArrowDown" ? 1 : -1;
        setMentionIndex(
          (index) =>
            (Math.min(index, mentionOptions.length - 1) + step + mentionOptions.length) %
            mentionOptions.length,
        );
        return;
      }
      if ((event.key === "Enter" && !event.shiftKey) || event.key === "Tab") {
        event.preventDefault();
        if (selectedMention) chooseMention(selectedMention.name);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setMention(undefined);
        return;
      }
    }
    if (event.key === "Enter" && !event.shiftKey && !isComposing) {
      event.preventDefault();
      void submit();
    }
  }

  function scrubDraft(event: PointerEvent<HTMLDivElement>) {
    const element = editorRef.current?.element;
    const thumb = scrollEdges.thumb;
    if (!element || !thumb) return;
    const track = event.currentTarget.getBoundingClientRect();
    const thumbHeight = track.height * thumb.size / 100;
    const position = (event.clientY - track.top - thumbHeight / 2) / (track.height - thumbHeight);
    element.scrollTop = Math.max(0, Math.min(1, position)) * (element.scrollHeight - element.clientHeight);
    setScrollEdges(getScrollEdges(element));
  }

  function handleScroll() {
    if (editorRef.current) setScrollEdges(getScrollEdges(editorRef.current.element));
  }

  return (
    <div
      // Opaque: the transcript scrolls underneath the composer, so its band
      // must hide it instead of showing text around and below the input.
      data-composer-band
      ref={bandRef}
      className="relative flex shrink-0 flex-col justify-end bg-background"
      style={{
        width: "calc(100% - var(--assistant-content-inset, 0px))",
        maxHeight: "min(50dvh, 480px)",
      }}
    >
      <div ref={surfaceRef} data-composer-surface className="relative shrink-0 bg-background px-4 sm:px-6">
        <div
          aria-hidden="true"
          data-composer-fade
          className="pointer-events-none absolute inset-x-0 bottom-full h-6 bg-gradient-to-t from-background to-transparent"
        />
        <div
          className="mx-auto w-full max-w-[var(--chat-content-width,758px)] pt-2"
          style={{ paddingBottom: "max(1rem, var(--sab))" }}
        >
          {controls}
          {active && allowActiveInput ? (
            <p
              data-steering-hint
              className="mb-1.5 text-11 text-muted-foreground"
            >
              Still replying. What you send now steers this reply; Stop ends it.
            </p>
          ) : null}
          <div
            data-composer-input
            onDragEnter={(event) => {
              if (!onFiles || !event.dataTransfer.types.includes("Files")) return;
              event.preventDefault();
              dragDepth.current += 1;
              if (!locked && !disabled && !sending) setDragging(true);
            }}
            onDragLeave={(event) => {
              event.preventDefault();
              dragDepth.current = Math.max(0, dragDepth.current - 1);
              if (!dragDepth.current) setDragging(false);
            }}
            onDragOver={(event) => {
              if (onFiles && event.dataTransfer.types.includes("Files")) {
                event.preventDefault();
                event.dataTransfer.dropEffect =
                  locked || disabled || sending ? "none" : "copy";
              }
            }}
            onDrop={(event) => {
              dragDepth.current = 0;
              setDragging(false);
              if (onFiles && event.dataTransfer.files.length) {
                event.preventDefault();
                if (!locked && !disabled && !sending)
                  onFiles(Array.from(event.dataTransfer.files));
              }
            }}
            onPasteCapture={(event) => {
              const files = event.clipboardData.files.length
                ? Array.from(event.clipboardData.files)
                : Array.from(event.clipboardData.items)
                    .map((item) => item.getAsFile())
                    .filter((file): file is File => Boolean(file));
              // A wall of text is attached as a file and referenced at the caret.
              if (
                onLongText &&
                !files.length &&
                !locked &&
                !disabled &&
                !sending
              ) {
                const pastedText = event.clipboardData.getData("text/plain");
                const handle = isLongPaste(pastedText)
                  ? onLongText(pastedText)
                  : undefined;
                if (handle) {
                  event.preventDefault();
                  insertAtCaret(`@${handle} `);
                  return;
                }
              }
              if (onFiles && files.length) {
                event.preventDefault();
                if (!locked && !disabled && !sending) {
                  onFiles(
                    files.map((file) => {
                      if (!isGenericPasteName(file.name)) return file;
                      pastedImages.current += 1;
                      const type = file.type || "image/png";
                      return new File(
                        [file],
                        `pasted-image-${pastedImages.current}.${pastedExtension(type)}`,
                        { type },
                      );
                    }),
                  );
                }
              }
            }}
            style={
              { "--composer-audio-level": audioLevel } as React.CSSProperties
            }
            data-audio-active={
              voiceActive || dictation.captureActive || undefined
            }
            data-audio-input={voiceActivity?.input ?? dictation.activity.input}
            data-audio-output={voiceActivity?.output ?? 0}
            className={cn(
              "composer-audio-surface relative rounded-xl border bg-card px-3 py-2 transition-colors focus-within:border-hairline-strong",
              dragging ? "border-primary ring-1 ring-primary" : "border-hairline",
            )}
          >
            {voiceControls}
            {expanded && (
              <div
                role="separator"
                aria-label="Resize composer"
                aria-orientation="horizontal"
                aria-valuemin={heightRange.min}
                aria-valuemax={heightRange.max}
                aria-valuenow={heightRange.current}
                tabIndex={0}
                title="Drag to set maximum height for all chats. Double-click to reset."
                className="group absolute -top-1.5 left-1/2 z-20 flex h-3 w-12 -translate-x-1/2 touch-none cursor-ns-resize items-center justify-center rounded-full focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                onPointerDown={(event) => {
                  if (event.button !== 0 || resizeStartRef.current) return;
                  event.preventDefault();
                  resizeStartRef.current = {
                    pointer: event.pointerId,
                    y: event.clientY,
                    height: editorRef.current?.element.getBoundingClientRect().height ?? MIN_TEXT_HEIGHT,
                    current: savedMaxHeight,
                  };
                  event.currentTarget.setPointerCapture(event.pointerId);
                  setResizing(true);
                }}
                onPointerMove={(event) => {
                  const start = resizeStartRef.current;
                  if (!start || start.pointer !== event.pointerId) return;
                  start.current = Math.round(Math.max(heightRange.min, Math.min(heightRange.max, start.height + start.y - event.clientY)));
                  setPreviewHeight(start.current);
                }}
                onPointerCancel={cancelHeightResize}
                onLostPointerCapture={cancelHeightResize}
                onPointerUp={(event) => {
                  if (resizeStartRef.current?.pointer !== event.pointerId) return;
                  setMaxTextHeight(resizeStartRef.current.current);
                  setResizedTextHeight(resizeStartRef.current.current);
                  cancelHeightResize();
                  if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
                }}
                onDoubleClick={() => {
                  setMaxTextHeight(COMPOSER_MAX_HEIGHT.default);
                  setResizedTextHeight(undefined);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Escape") { cancelHeightResize(); return; }
                  if (!["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) return;
                  event.preventDefault();
                  const current = heightRange.current;
                  const next = event.key === "Home" ? heightRange.min : event.key === "End" ? heightRange.max : current + (event.key === "ArrowUp" ? 24 : -24);
                  const height = Math.max(heightRange.min, Math.min(heightRange.max, next));
                  setMaxTextHeight(height);
                  setResizedTextHeight(height);
                }}
              >
                <span
                  aria-hidden
                  className="h-1 w-7 rounded-full bg-border transition-colors group-hover:bg-muted-foreground/60 group-focus-visible:bg-muted-foreground/60"
                />
              </div>
            )}
            {dragging && (
              <div
                role="status"
                className="pointer-events-none absolute inset-0 z-30 flex items-center justify-center rounded-xl bg-card/95 text-12 font-medium"
              >
                Drop files to attach
              </div>
            )}
            {attachments}
            {dictation.review}
            {recipients}
            <div className="flex items-start gap-1.5">
              <div
                ref={composerRef}
                data-composer-editor
                className={`relative flex min-w-0 flex-1 gap-x-1.5 gap-y-0.5 ${
                  expanded
                    ? "flex-col items-stretch"
                    : "composer-compact items-start"
                }`}
              >
                {!expanded && attachmentButton && (
                  <div
                    data-composer-attachment
                    ref={attachmentRef}
                    className="shrink-0"
                  >
                    {attachmentButton}
                  </div>
                )}
                {mentionOpen ? (
                  <ul
                    id={mentionListId}
                    role="listbox"
                    aria-label={
                      attachmentMentions?.length
                        ? "Mention an agent or attachment"
                        : "Mention an agent"
                    }
                    className="absolute bottom-full left-0 z-20 mb-2 w-64 max-w-full space-y-0.5 rounded-xl border border-border bg-popover p-1.5 shadow-lg shadow-primary/5"
                  >
                    {mentionOptions.map((option) => {
                      const selected = option.id === selectedMention?.id;
                      return (
                        <li
                          key={option.id}
                          id={`${mentionListId}-${option.id}`}
                          role="option"
                          aria-selected={selected}
                          // Keep focus (and the caret) in the textarea.
                          onMouseDown={(event) => event.preventDefault()}
                          onClick={() => chooseMention(option.name)}
                          className={cn(
                            "flex cursor-pointer items-center gap-2 rounded-md px-2.5 py-1.5 text-12 text-foreground",
                            selected ? "bg-overlay-strong" : "hover:bg-overlay",
                          )}
                        >
                          {option.type === "agent" ? (
                            <>
                              <AgentAvatar agent={option.agent} size="sm" />
                              <span className="min-w-0 flex-1 truncate">
                                {option.agent.display_name?.trim() || option.name}
                                {option.agent.display_name?.trim() ? (
                                  <span className="ml-1.5 text-11 text-text-tertiary">
                                    @{option.name}
                                  </span>
                                ) : null}
                              </span>
                              <span className="shrink-0 text-10 text-text-tertiary">
                                {option.agent.kind === "nyxbot"
                                  ? "Personal agent"
                                  : "Specialist"}
                              </span>
                            </>
                          ) : (
                            <>
                              {option.attachment.kind === "image" ? (
                                <ImageIcon
                                  aria-hidden
                                  className="h-4 w-4 shrink-0 text-muted-foreground"
                                />
                              ) : (
                                <FileText
                                  aria-hidden
                                  className="h-4 w-4 shrink-0 text-muted-foreground"
                                />
                              )}
                              <span className="min-w-0 flex-1 truncate">
                                <span className="font-mono">@{option.name}</span>
                                <span className="ml-1.5 text-11 text-text-tertiary">
                                  {option.attachment.label}
                                </span>
                              </span>
                            </>
                          )}
                        </li>
                      );
                    })}
                  </ul>
                ) : null}
                <span
                  ref={textMeasureRef}
                  data-composer-text-measure
                  aria-hidden
                  className="pointer-events-none absolute invisible inline-block w-max whitespace-pre text-base sm:text-13 leading-relaxed"
                >
                  {tokenSegments(content, tokens).map((part, index) =>
                    part.token ? (
                      <span
                        key={index}
                        className="composer-mention"
                        data-composer-mention={part.token.kind}
                      >
                        {part.token.kind === "agent" ? "@" : ""}
                        {part.token.label}
                      </span>
                    ) : (
                      part.text
                    ),
                  )}
                </span>
                <div
                  className={cn(
                    "composer-text-region relative min-w-0 flex-1",
                    dictation.active && "hidden",
                  )}
                >
                  <ComposerEditor
                    key={`${ownerUserId ?? ""}:${draftKey ?? ""}`}
                    ref={editorRef}
                    value={content}
                    tokens={tokens}
                    expanded={expanded}
                    {...(mentions
                      ? {
                          role: "combobox",
                          "aria-autocomplete": "list" as const,
                          "aria-expanded": mentionOpen,
                          "aria-controls": mentionListId,
                          "aria-activedescendant": selectedMention
                            ? `${mentionListId}-${selectedMention.id}`
                            : undefined,
                          "aria-label": placeholder ?? "Message",
                        }
                      : {})}
                    onChange={(value) => {
                      updateContent(value);
                      if (editorRef.current) syncMention(editorRef.current);
                      if (!composingRef.current) scheduleDraftSave();
                    }}
                    onSelectionChange={syncMention}
                    onBlur={() => setMention(undefined)}
                    onKeyDown={handleKeyDown}
                    onCompositionStart={() => {
                      composingRef.current = true;
                      cancelScheduledSave();
                    }}
                    onCompositionEnd={() => {
                      composingRef.current = false;
                      updateContent(
                        editorRef.current?.value ?? contentRef.current,
                      );
                      scheduleDraftSave();
                    }}
                    onScroll={handleScroll}
                    disabled={locked || disabled || dictation.active}
                    style={{ transitionDuration: resizing ? "0ms" : undefined }}
                    placeholder={
                      locked
                        ? "Assistant is working..."
                        : disabled
                          ? "This conversation is read-only."
                          : allowActiveInput
                            ? (activePlaceholder ?? "Steer active task...")
                            : (placeholder ?? "Message NyxID Assistant...")
                    }
                    className="assistant-scrollbar composer-scrollbar block min-h-8 w-full resize-none overflow-hidden overscroll-contain bg-transparent px-0 py-1 text-base sm:text-13 leading-relaxed text-foreground outline-none transition-[height] duration-150 ease-out placeholder:text-text-tertiary disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none"
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
                  {scrollEdges.thumb && (
                    <div
                      data-composer-scrollbar
                      aria-hidden="true"
                      className="absolute -right-2 top-1 bottom-1 w-1.5 touch-none cursor-default rounded-full"
                      onPointerDown={(event) => {
                        event.preventDefault();
                        event.currentTarget.setPointerCapture(event.pointerId);
                        scrubDraft(event);
                      }}
                      onPointerMove={(event) => {
                        if (
                          event.currentTarget.hasPointerCapture(event.pointerId)
                        )
                          scrubDraft(event);
                      }}
                      onPointerUp={(event) => {
                        if (
                          event.currentTarget.hasPointerCapture(event.pointerId)
                        )
                          event.currentTarget.releasePointerCapture(
                            event.pointerId,
                          );
                      }}
                    >
                      <div
                        className="absolute inset-x-0 rounded-full bg-muted-foreground/50"
                        style={{
                          height: `${scrollEdges.thumb.size}%`,
                          top: `${scrollEdges.thumb.top}%`,
                        }}
                      />
                    </div>
                  )}
                </div>
                <div
                  data-composer-footer
                  className={cn(
                    "flex shrink-0 items-center gap-1.5",
                    expanded && "w-full",
                  )}
                >
                  {expanded && attachmentButton && (
                    <div ref={attachmentRef} className="shrink-0">
                      {attachmentButton}
                    </div>
                  )}
                  {expanded && <div className="min-w-0 flex-1" />}
                  {footerControls && (
                    <div
                      data-composer-settings
                      ref={settingsRef}
                      className="min-w-0 max-w-[72px] sm:max-w-[160px]"
                    >
                      {footerControls}
                    </div>
                  )}
                  <div
                    ref={controlsRef}
                    className={`flex shrink-0 items-center gap-1.5 ${expanded ? "self-end" : ""}`}
                  >
                    {dictation.button}
                    {showVoice && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        disabled={
                          (!voiceActive && sending) ||
                          disabled ||
                          dictation.active
                        }
                        className={cn(
                          "[&_svg]:size-5",
                          voiceActive &&
                            "text-primary hover:text-primary focus-visible:text-primary active:text-primary",
                        )}
                        onClick={() => {
                          dictation.releaseMicrophone();
                          flushDraft();
                          onVoice?.();
                        }}
                        aria-label="Open voice call"
                        aria-pressed={voiceActive}
                      >
                        <AudioLines />
                      </Button>
                    )}
                    {active && allowActiveInput && (
                      <Button
                        type="button"
                        variant="outline"
                        size="icon"
                        onClick={() => void onStop()}
                        disabled={stopDisabled}
                        aria-label="Stop assistant turn"
                        title="Stop this reply"
                      >
                        <Square className="fill-current" />
                      </Button>
                    )}
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
                    ) : hasDraftContent ? (
                      <Button
                        type="button"
                        variant="primary"
                        // While a reply runs, sending steers it: say so on the button.
                        size={active && allowActiveInput ? "default" : "icon"}
                        className={
                          active && allowActiveInput ? "gap-1.5" : undefined
                        }
                        title={
                          active && allowActiveInput
                            ? "Steer this reply"
                            : undefined
                        }
                        disabled={
                          (!content.trim() && !hasAttachments) ||
                          uploadBlocked ||
                          sending ||
                          disabled ||
                          dictation.active
                        }
                        onClick={() => void submit()}
                        aria-label={
                          sendLabel ??
                          (allowActiveInput
                            ? "Send steering instruction"
                            : "Send message")
                        }
                      >
                        <ArrowUp />
                        {active && allowActiveInput ? "Steer" : null}
                      </Button>
                    ) : null}
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
