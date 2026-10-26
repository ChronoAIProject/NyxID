import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { CHAT_CONTENT_WIDTH, clampChatContentWidth, useThemeStore } from "@/stores/theme-store";
import { cn } from "@/lib/utils";
import { ChatHeaderCompactContext } from "./chat-header-context";

export function ChatContentLayout({ children }: { readonly children: ReactNode }) {
  const savedWidth = useThemeStore((state) => state.chatContentWidth);
  const setWidth = useThemeStore((state) => state.setChatContentWidth);
  const [preview, setPreview] = useState<number | null>(null);
  const [available, setAvailable] = useState<number>(CHAT_CONTENT_WIDTH.max);
  const [scrollbar, setScrollbar] = useState(0);
  const [composerInset, setComposerInset] = useState<number | null>(null);
  const [headerCompact, setHeaderCompact] = useState(false);
  const compactRef = useRef(false);
  const headerAnchor = useRef<{ element: HTMLElement; scrollTop: number; viewportTop: number } | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const bounds = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pointer: number; x: number; width: number; current: number } | null>(null);
  const width = preview ?? savedWidth;
  const maximum = Math.max(CHAT_CONTENT_WIDTH.min, Math.min(CHAT_CONTENT_WIDTH.max, available));
  const visibleWidth = Math.min(width, maximum);

  useLayoutEffect(() => {
    const element = root.current;
    const boundary = bounds.current;
    if (!element || !boundary || typeof ResizeObserver === "undefined") return;
    let transcript: HTMLElement | null = null;
    let composer: HTMLElement | null = null;
    function updateHeader() {
      const panels = element!.querySelector<HTMLElement>("[data-chat-thread-panels]");
      if (!transcript || !panels || headerAnchor.current) return;
      const next = compactRef.current
        ? transcript.scrollTop > 24
        : transcript.scrollTop > Math.max(160, panels.offsetHeight + 96);
      if (next === compactRef.current) return;
      headerAnchor.current = { element: transcript, scrollTop: transcript.scrollTop, viewportTop: transcript.getBoundingClientRect().top };
      compactRef.current = next;
      setHeaderCompact(next);
    }
    function scroll(event: Event) {
      if (event.target === transcript) updateHeader();
    }
    const observer = new ResizeObserver(() => {
      setAvailable(boundary.clientWidth);
      setScrollbar(transcript ? transcript.offsetWidth - transcript.clientWidth : 0);
      setComposerInset(composer ? Math.max(0, element.getBoundingClientRect().bottom - composer.getBoundingClientRect().top) : null);
      updateHeader();
    });
    function findTranscript() {
      const nextComposer = element!.querySelector<HTMLElement>("[data-composer-surface], [data-composer-band]:not(:has([data-composer-surface]))");
      if (nextComposer !== composer) {
        if (composer) observer.unobserve(composer);
        composer = nextComposer;
        if (composer) observer.observe(composer);
        else setComposerInset(null);
      }
      const next = element!.querySelector<HTMLElement>("[data-chat-transcript]");
      if (next === transcript) return;
      if (transcript) observer.unobserve(transcript);
      transcript = next;
      if (transcript) observer.observe(transcript);
      else setScrollbar(0);
    }
    const mutations = new MutationObserver(() => {
      if (!transcript || !element.contains(transcript) || !composer || !element.contains(composer)) findTranscript();
    });
    mutations.observe(element, { childList: true, subtree: true });
    element.addEventListener("scroll", scroll, { capture: true, passive: true });
    observer.observe(boundary);
    findTranscript();
    return () => {
      observer.disconnect();
      mutations.disconnect();
      element.removeEventListener("scroll", scroll, true);
    };
  }, []);

  useLayoutEffect(() => {
    const anchor = headerAnchor.current;
    headerAnchor.current = null;
    if (!anchor || !root.current?.contains(anchor.element)) return;
    // Removing the header moves the viewport's top. Offset the scroll by that
    // same amount so the text being read stays at its screen position.
    const shift = anchor.element.getBoundingClientRect().top - anchor.viewportTop;
    anchor.element.scrollTop = !headerCompact && anchor.scrollTop <= 1
      ? 0
      : anchor.scrollTop + shift;
  }, [headerCompact]);

  function cancel() {
    drag.current = null;
    setPreview(null);
  }
  function bound(value: number) {
    return Math.min(maximum, clampChatContentWidth(value));
  }

  return (
    <ChatHeaderCompactContext value={headerCompact}>
      <div
        ref={root}
        data-chat-content-layout=""
        className="relative h-full min-h-0"
        style={{ "--chat-content-width": `${width}px`, "--assistant-scrollbar-width": `${scrollbar}px`, "--assistant-content-inset": "calc(var(--assistant-scrollbar-width) + var(--chat-index-width, 0px))", "--assistant-composer-inset": composerInset === null ? undefined : `${composerInset}px` } as CSSProperties}
      >
        {children}
        <div ref={bounds} className="pointer-events-none absolute inset-y-0 left-4 right-[calc(var(--spacing)*4+var(--assistant-content-inset))] sm:left-6 sm:right-[calc(var(--spacing)*6+var(--assistant-content-inset))]">
          <div className="relative mx-auto h-full w-full max-w-[var(--chat-content-width)]">
            {(["left", "right"] as const).map((side) => (
              <div
                key={side}
                className={cn(
                  "group absolute top-0 bottom-4 z-30 hidden w-5 md:block",
                  side === "left" ? "-left-2.5" : "-right-2.5",
                  available < CHAT_CONTENT_WIDTH.min && "md:hidden",
                )}
              >
                <div
                  role="separator"
                  aria-label={`Resize chat content from ${side}`}
                  aria-orientation="vertical"
                  aria-valuemin={CHAT_CONTENT_WIDTH.min}
                  aria-valuemax={maximum}
                  aria-valuenow={visibleWidth}
                  aria-valuetext={`${visibleWidth} pixels`}
                  tabIndex={0}
                  title="Drag to resize chat · double-click to reset"
                  className="pointer-events-auto relative h-5 w-full touch-none cursor-col-resize outline-none"
                  onPointerDown={(event) => {
                    if (event.button !== 0 || drag.current) return;
                    event.preventDefault();
                    event.currentTarget.setPointerCapture(event.pointerId);
                    drag.current = { pointer: event.pointerId, x: event.clientX, width: visibleWidth, current: visibleWidth };
                    setPreview(visibleWidth);
                  }}
                  onPointerMove={(event) => {
                    const start = drag.current;
                    if (!start || start.pointer !== event.pointerId) return;
                    const next = bound(start.width + (event.clientX - start.x) * (side === "right" ? 2 : -2));
                    start.current = next;
                    setPreview(next);
                  }}
                  onPointerUp={(event) => {
                    if (drag.current?.pointer !== event.pointerId) return;
                    setWidth(drag.current.current);
                    cancel();
                  }}
                  onPointerCancel={cancel}
                  onLostPointerCapture={cancel}
                  onDoubleClick={() => setWidth(CHAT_CONTENT_WIDTH.default)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") { cancel(); return; }
                    const delta = { ArrowLeft: -16, ArrowRight: 16 }[event.key];
                    if (delta !== undefined) {
                      event.preventDefault();
                      setWidth(bound(visibleWidth + delta * (side === "right" ? 1 : -1)));
                    } else if (event.key === "Home" || event.key === "End") {
                      event.preventDefault();
                      setWidth(event.key === "Home" ? CHAT_CONTENT_WIDTH.min : maximum);
                    }
                  }}
                >
                  <svg
                    aria-hidden="true"
                    viewBox="0 0 10 6"
                    className="pointer-events-none absolute top-0 left-1/2 h-1.5 w-2.5 -translate-x-1/2 fill-hairline-strong transition-colors group-hover:fill-foreground group-focus-within:fill-input-focus motion-reduce:transition-none"
                  >
                    <path d="M0 0h10L5 6Z" />
                  </svg>
                </div>
                <span
                  aria-hidden="true"
                  data-chat-width-guide=""
                  className={cn(
                    "pointer-events-none absolute top-2 bottom-0 left-1/2 w-px -translate-x-1/2 bg-input-focus transition-opacity duration-150 motion-reduce:transition-none",
                    preview !== null ? "opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100",
                  )}
                />
              </div>
            ))}
          </div>
        </div>
      </div>
    </ChatHeaderCompactContext>
  );
}
