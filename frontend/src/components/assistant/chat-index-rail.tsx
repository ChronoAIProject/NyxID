import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import { useScrollFades } from "@/hooks/use-scroll-fades";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import { sanitizeAssistantMessageContent } from "@/lib/assistant/chat-content";
import { cn } from "@/lib/utils";

interface ChatIndexRailProps {
  readonly messages: readonly ChatMessage[];
  readonly scrollRef: RefObject<HTMLDivElement | null>;
  readonly bottomInset: number;
  readonly onNavigate: (messageIndex: number) => void;
}
interface IndexEntry {
  id: string;
  messageIndex: number;
  title: string;
  role: "user" | "assistant";
  author: string;
}
interface RailDrag {
  pointer: number;
  last: number;
  top: number;
  height: number;
  offset: number;
  pitch: number;
  y: number;
  time: number;
}

export function ChatIndexRail({ messages, ...props }: ChatIndexRailProps) {
  const entries = useMemo(() => messages.flatMap<IndexEntry>((message, messageIndex) => {
    if (message.role !== "user" && message.role !== "assistant") return [];
    const firstLine = sanitizeAssistantMessageContent(message.content)
      .split(/\r?\n/).find((line) => line.trim())?.trim();
    const title = firstLine?.replace(/^#{1,6}\s+/, "").slice(0, 200)
      || message.images?.[0]?.label || message.error
      || (message.status === "streaming" ? "Working…" : "Message");
    return [{ id: message.id, messageIndex, title, role: message.role === "user" ? "user" : "assistant",
      author: message.authorName || (message.role === "user" ? "You" : "Assistant") }];
  }), [messages]);
  return entries.length < 2 ? null : <IndexRail entries={entries} {...props} />;
}

function IndexRail({ entries, scrollRef, bottomInset, onNavigate }: Omit<ChatIndexRailProps, "messages"> & { readonly entries: readonly IndexEntry[] }) {
  const container = useRef<HTMLDivElement>(null);
  const pitchMeasure = useRef<HTMLSpanElement>(null);
  const rail = useRef<HTMLElement>(null);
  const [layout, setLayout] = useState({ height: 480, pitch: 7 });
  const [viewportScroll, setViewportScroll] = useState(0);
  const [activeId, setActiveId] = useState<string>();
  const [keyboardIndex, setKeyboardIndex] = useState<number | null>(null);
  const [hover, setHover] = useState<{ id: string; y: number } | null>(null);
  const dragging = useRef<RailDrag | null>(null);
  const dragFrame = useRef(0);
  const active = Math.max(0, entries.findIndex((entry) => entry.id === activeId));
  const totalHeight = entries.length * layout.pitch;
  const centerOffset = Math.max(0, (layout.height - totalHeight) / 2);
  const firstVisible = Math.max(0, Math.min(entries.length - 1, Math.floor(viewportScroll / layout.pitch)));
  const lastVisible = Math.min(entries.length - 1, Math.ceil((viewportScroll + layout.height) / layout.pitch) - 1);
  const first = Math.max(0, firstVisible - 8);
  const end = Math.min(entries.length, lastVisible + 9);
  const focusIndex = Math.max(firstVisible, Math.min(lastVisible, keyboardIndex ?? active));
  const hoveredEntry = hover ? entries.find((entry) => entry.id === hover.id) : undefined;
  useScrollFades(container);

  useLayoutEffect(() => {
    const element = container.current;
    const measureElement = pitchMeasure.current;
    if (!element || !measureElement || typeof ResizeObserver === "undefined") return;
    let frame = 0;
    const measure = () => {
      frame = 0;
      // Keep the floating rail below the toolbar, including wrapped headers.
      const controls = scrollRef.current?.closest("[data-chat-content-layout]")?.querySelector("[data-chat-header-controls]");
      const parent = element.parentElement;
      element.style.top = `${controls && parent ? Math.max(0, controls.getBoundingClientRect().bottom - parent.getBoundingClientRect().top + 8) : 0}px`;
      const pitch = measureElement.getBoundingClientRect().height || 7;
      const height = element.clientHeight;
      setLayout((current) => current.height === height && current.pitch === pitch ? current : { height, pitch });
    };
    // Updating top also resizes this observed viewport. Write in the next
    // frame, outside ResizeObserver delivery, including during header motion.
    const observer = new ResizeObserver(() => {
      if (!frame) frame = requestAnimationFrame(measure);
    });
    observer.observe(element);
    observer.observe(measureElement);
    const controls = scrollRef.current?.closest("[data-chat-content-layout]")?.querySelector("[data-chat-header-controls]");
    if (controls) observer.observe(controls);
    measure();
    return () => { observer.disconnect(); cancelAnimationFrame(frame); };
  }, [scrollRef]);

  const reveal = useCallback((index: number) => {
    const element = rail.current;
    if (!element) return;
    const inset = Math.min(layout.pitch * 4, layout.height / 4);
    const top = index * layout.pitch;
    const bottom = top + layout.pitch;
    const maximum = Math.max(0, entries.length * layout.pitch - layout.height);
    if (top < element.scrollTop + inset) element.scrollTop = Math.max(0, top - inset);
    else if (bottom > element.scrollTop + layout.height - inset) element.scrollTop = Math.min(maximum, bottom - layout.height + inset);
    setViewportScroll(element.scrollTop);
  }, [entries.length, layout]);

  useLayoutEffect(() => {
    if (!dragging.current) reveal(active);
  }, [active, reveal]);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    let frame = 0;
    const update = () => {
      const top = element.getBoundingClientRect().top + 32;
      const rows = element.querySelectorAll<HTMLElement>("[data-chat-message-index]");
      let messageIndex = Number(rows[0]?.dataset.chatMessageIndex ?? 0);
      for (const row of rows) {
        if (row.getBoundingClientRect().top > top) break;
        messageIndex = Number(row.dataset.chatMessageIndex);
      }
      const atEnd = element.scrollHeight - element.clientHeight - element.scrollTop <= 2;
      let entry = entries[0];
      for (const item of entries) {
        if (item.messageIndex > messageIndex) break;
        entry = item;
      }
      if (atEnd) entry = entries.at(-1);
      setActiveId(entry?.id);
    };
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(update);
    };
    element.addEventListener("scroll", schedule, { passive: true });
    const observer = new ResizeObserver(schedule);
    observer.observe(element);
    if (element.firstElementChild) observer.observe(element.firstElementChild);
    schedule();
    return () => {
      cancelAnimationFrame(frame);
      element.removeEventListener("scroll", schedule);
      observer.disconnect();
    };
  }, [entries, scrollRef, bottomInset]);

  useEffect(() => () => cancelAnimationFrame(dragFrame.current), []);

  useLayoutEffect(() => {
    if (keyboardIndex === null) return;
    rail.current?.querySelector<HTMLButtonElement>(`[data-chat-index-target="${keyboardIndex}"]`)?.focus({ preventScroll: true });
  }, [keyboardIndex, first, end]);

  function preview(index: number, element: HTMLElement) {
    const bounds = container.current?.getBoundingClientRect();
    if (!bounds) return;
    const box = element.getBoundingClientRect();
    setHover({ id: entries[index]!.id, y: Math.max(40, Math.min(bounds.height - 40, box.top + box.height / 2 - bounds.top)) });
  }
  function navigate(index: number) {
    const entry = entries[index];
    if (entry) onNavigate(entry.messageIndex);
  }
  function selectAtPointer(drag: RailDrag) {
    const element = rail.current;
    if (!element) return;
    const index = Math.max(0, Math.min(entries.length - 1, Math.floor((element.scrollTop + drag.y - drag.top - drag.offset) / drag.pitch)));
    if (index === drag.last) return;
    drag.last = index;
    const tick = element.querySelector<HTMLElement>(`[data-chat-index-target="${index}"]`);
    if (tick) preview(index, tick);
    navigate(index);
  }
  function finishDrag() {
    const drag = dragging.current;
    dragging.current = null;
    cancelAnimationFrame(dragFrame.current);
    if (drag) reveal(drag.last);
    setHover(null);
  }
  return (
    <div ref={container} data-chat-index-container="" className="pointer-events-none absolute top-0 right-[var(--assistant-scrollbar-width,0px)] z-30 hidden w-[var(--chat-index-track-width)] items-center justify-end md:flex" style={{ bottom: `var(--assistant-composer-inset, ${bottomInset}px)` }}>
      <span ref={pitchMeasure} aria-hidden="true" data-chat-index-pitch="" className="pointer-events-none absolute w-0" style={{ height: "var(--chat-index-pitch)" }} />
      <nav
        ref={rail}
        data-chat-index-rail=""
        data-scroll-fade=""
        aria-label="Conversation index"
        className="pointer-events-auto scrollbar-none h-full w-full touch-none select-none overflow-x-hidden overflow-y-auto overscroll-contain scroll-auto"
        onScroll={(event) => { setViewportScroll(event.currentTarget.scrollTop); setHover(null); }}
        onPointerLeave={() => { if (!dragging.current) setHover(null); }}
        onBlur={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            setKeyboardIndex(null);
            setHover(null);
          }
        }}
        onKeyDown={(event) => {
          const index = Number((event.target as HTMLElement).closest<HTMLElement>("[data-chat-index-target]")?.dataset.chatIndexTarget);
          if (!Number.isFinite(index)) return;
          const page = Math.max(1, Math.floor(layout.height / layout.pitch) - 1);
          const next = { ArrowUp: index - 1, ArrowDown: index + 1, Home: 0, End: entries.length - 1, PageUp: index - page, PageDown: index + page }[event.key];
          if (next === undefined) return;
          event.preventDefault();
          const bounded = Math.max(0, Math.min(entries.length - 1, next));
          reveal(bounded);
          setKeyboardIndex(bounded);
          navigate(bounded);
        }}
        onPointerDown={(event) => {
          const target = (event.target as HTMLElement).closest<HTMLElement>("[data-chat-index-target]");
          if (event.button !== 0 || !target) return;
          event.preventDefault();
          const index = Number(target.dataset.chatIndexTarget);
          const bounds = event.currentTarget.getBoundingClientRect();
          event.currentTarget.setPointerCapture(event.pointerId);
          // Snapshot the viewport so collapsing the toolbar cannot move drag targets.
          dragging.current = { pointer: event.pointerId, last: index, top: bounds.top, height: bounds.height, offset: centerOffset, pitch: layout.pitch, y: event.clientY, time: event.timeStamp };
          setKeyboardIndex(null);
          preview(index, target);
          navigate(index);
          const advance = (time: number) => {
            const drag = dragging.current;
            const element = rail.current;
            if (!drag || !element) return;
            const zone = Math.min(drag.pitch * 4, drag.height / 4);
            const y = drag.y - drag.top;
            const speed = y < zone ? Math.max(-2, (y - zone) / zone) : y > drag.height - zone ? Math.min(2, (y - drag.height + zone) / zone) : 0;
            const elapsed = Math.min(64, time - drag.time);
            drag.time = time;
            if (speed) {
              element.scrollTop += speed * drag.pitch * elapsed / 50;
              setViewportScroll(element.scrollTop);
              selectAtPointer(drag);
            }
            dragFrame.current = requestAnimationFrame(advance);
          };
          dragFrame.current = requestAnimationFrame(advance);
        }}
        onPointerMove={(event) => {
          const drag = dragging.current;
          if (!drag || drag.pointer !== event.pointerId) return;
          drag.y = event.clientY;
          selectAtPointer(drag);
        }}
        onPointerUp={finishDrag}
        onPointerCancel={finishDrag}
        onLostPointerCapture={finishDrag}
      >
        <div className="relative min-h-full w-full" style={{ height: totalHeight }}>
          {entries.slice(first, end).map((entry, offset) => {
            const index = first + offset;
            const selected = entry.id === activeId;
            return (
              <button
                key={entry.id}
                type="button"
                data-chat-index-target={index}
                data-message-id={entry.id}
                data-message-role={entry.role}
                aria-label={`Jump to message ${index + 1}: ${entry.author} — ${entry.title}`}
                aria-current={selected ? "location" : undefined}
                tabIndex={index === focusIndex ? 0 : -1}
                className="group absolute left-0 flex w-full cursor-pointer items-center justify-end pr-[calc(var(--chat-index-tick-width)/4)] outline-none focus-visible:rounded-sm focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-muted-foreground"
                style={{ height: layout.pitch, top: centerOffset + index * layout.pitch }}
                onPointerEnter={(event) => { if (event.pointerType !== "touch" && !dragging.current) preview(index, event.currentTarget); }}
                onFocus={(event) => { reveal(index); preview(index, event.currentTarget); }}
                onClick={(event) => { if (event.detail === 0) navigate(index); }}
              >
                <span aria-hidden="true" className={cn(
                  "h-[var(--chat-index-tick-thickness)] shrink-0 rounded-full bg-current transition-[color,opacity] duration-150 motion-reduce:transition-none",
                  selected || entry.role === "assistant" ? "w-[var(--chat-index-tick-width)]" : "w-[calc(var(--chat-index-tick-width)/2)]",
                  selected ? "text-nyx-secondary-400 opacity-100" : "text-text-tertiary opacity-50 group-hover:text-muted-foreground group-hover:opacity-100 group-focus-visible:text-muted-foreground group-focus-visible:opacity-100 group-active:text-muted-foreground group-active:opacity-100",
                )} />
              </button>
            );
          })}
        </div>
      </nav>
      {hover && hoveredEntry ? (
        <div role="tooltip" className="absolute right-full mr-2 w-max max-w-72 -translate-y-1/2 rounded-lg border border-hairline bg-popover px-3 py-2 text-11 text-foreground shadow-sm" style={{ top: hover.y }}>
          <p className="mb-1 font-medium text-text-tertiary">{hoveredEntry.author}</p>
          <p className="line-clamp-3 whitespace-pre-wrap break-words">{hoveredEntry.title}</p>
        </div>
      ) : null}
    </div>
  );
}
