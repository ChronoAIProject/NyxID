import { useEffect, useLayoutEffect, useMemo, useRef, useState, type RefObject } from "react";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import { sanitizeAssistantMessageContent } from "@/lib/assistant/chat-content";
import { cn } from "@/lib/utils";

export function ChatIndexRail({ messages, scrollRef, bottomInset, onNavigate }: {
  readonly messages: readonly ChatMessage[];
  readonly scrollRef: RefObject<HTMLDivElement | null>;
  readonly bottomInset: number;
  readonly onNavigate: (messageIndex: number) => void;
}) {
  const entries = useMemo(() => messages.flatMap((message, messageIndex) => {
    if (message.role !== "user" && message.role !== "assistant") return [];
    const firstLine = sanitizeAssistantMessageContent(message.content)
      .split(/\r?\n/).find((line) => line.trim())?.trim();
    const title = firstLine?.replace(/^#{1,6}\s+/, "").slice(0, 200)
      || message.images?.[0]?.label || message.error
      || (message.status === "streaming" ? "Working…" : "Message");
    return [{ id: message.id, messageIndex, title, role: message.role,
      author: message.authorName || (message.role === "user" ? "You" : "Assistant") }];
  }), [messages]);
  const container = useRef<HTMLDivElement>(null);
  const pitchMeasure = useRef<HTMLSpanElement>(null);
  const rail = useRef<HTMLElement>(null);
  const ticks = useRef<HTMLDivElement>(null);
  const [layout, setLayout] = useState({ capacity: 60, pitch: 8 });
  const [activeId, setActiveId] = useState<string>();
  const [browseStart, setBrowseStart] = useState<number | null>(null);
  const [keyboardIndex, setKeyboardIndex] = useState<number | null>(null);
  const [hover, setHover] = useState<{ id: string; y: number } | null>(null);
  const dragging = useRef<{ pointer: number; first: number; last: number; top: number; pitch: number; count: number } | null>(null);
  const active = Math.max(0, entries.findIndex((entry) => entry.id === activeId));
  const count = Math.min(layout.capacity, entries.length);
  const maxFirst = Math.max(0, entries.length - count);
  const followFirst = Math.max(0, Math.min(maxFirst, active - Math.floor(count / 2)));
  const first = Math.max(0, Math.min(maxFirst, browseStart ?? followFirst));
  const focusIndex = Math.max(first, Math.min(first + count - 1, keyboardIndex ?? active));
  const hoveredEntry = hover ? entries.find((entry) => entry.id === hover.id) : undefined;

  useLayoutEffect(() => {
    const element = container.current;
    const measureElement = pitchMeasure.current;
    if (!element || !measureElement || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      const regularPitch = measureElement.getBoundingClientRect().height || 8;
      // Round down to browser layout precision so fitting ticks never overflow.
      const fittedPitch = Math.floor(element.clientHeight / entries.length * 64) / 64;
      const pitch = Math.min(regularPitch, Math.max(regularPitch * 5 / 8, fittedPitch));
      const capacity = Math.max(2, Math.floor(element.clientHeight / pitch));
      setLayout((current) => current.capacity === capacity && current.pitch === pitch ? current : { capacity, pitch });
    };
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    observer.observe(measureElement);
    measure();
    return () => observer.disconnect();
  }, [entries.length]);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element || entries.length < 2) return;
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
    const scroll = () => {
      if (!dragging.current) setBrowseStart(null);
      schedule();
    };
    element.addEventListener("scroll", scroll, { passive: true });
    const observer = new ResizeObserver(schedule);
    observer.observe(element);
    if (element.firstElementChild) observer.observe(element.firstElementChild);
    schedule();
    return () => {
      cancelAnimationFrame(frame);
      element.removeEventListener("scroll", scroll);
      observer.disconnect();
    };
  }, [entries, scrollRef, bottomInset]);

  useEffect(() => {
    const element = rail.current;
    if (!element || !maxFirst) return;
    const wheel = (event: WheelEvent) => {
      if (!event.deltaY) return;
      event.preventDefault();
      const step = Math.sign(event.deltaY) * Math.min(12, Math.max(1, Math.ceil(Math.abs(event.deltaY) / 16)));
      setBrowseStart((current) => Math.max(0, Math.min(maxFirst, (current ?? followFirst) + step)));
      setHover(null);
    };
    element.addEventListener("wheel", wheel, { passive: false });
    return () => element.removeEventListener("wheel", wheel);
  }, [maxFirst, followFirst]);

  useLayoutEffect(() => {
    if (keyboardIndex === null) return;
    rail.current?.querySelector<HTMLButtonElement>(`[data-chat-index-target="${keyboardIndex}"]`)?.focus({ preventScroll: true });
  }, [keyboardIndex, first]);

  if (entries.length < 2) return null;

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
  return (
    <div ref={container} className="pointer-events-none absolute top-0 right-[var(--assistant-scrollbar-width,0px)] z-30 hidden w-7 items-center justify-end md:flex" style={{ bottom: `var(--assistant-composer-inset, ${bottomInset}px)` }}>
      <span ref={pitchMeasure} aria-hidden="true" className="pointer-events-none absolute h-2 w-0" />
      <nav
        ref={rail}
        data-chat-index-rail=""
        data-scroll-edges={[first > 0 && "top", first < maxFirst && "bottom"].filter(Boolean).join(" ")}
        aria-label="Conversation index"
        className="pointer-events-auto flex h-full w-full touch-none select-none flex-col items-end justify-center overflow-hidden"
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
          const next = { ArrowUp: index - 1, ArrowDown: index + 1, Home: 0, End: entries.length - 1, PageUp: index - count, PageDown: index + count }[event.key];
          if (next === undefined) return;
          event.preventDefault();
          const bounded = Math.max(0, Math.min(entries.length - 1, next));
          setBrowseStart(Math.max(0, Math.min(maxFirst, bounded - Math.floor(count / 2))));
          setKeyboardIndex(bounded);
          navigate(bounded);
        }}
        onPointerDown={(event) => {
          const target = (event.target as HTMLElement).closest<HTMLElement>("[data-chat-index-target]");
          if (event.button !== 0 || !target) return;
          event.preventDefault();
          const index = Number(target.dataset.chatIndexTarget);
          const bounds = ticks.current!.getBoundingClientRect();
          event.currentTarget.setPointerCapture(event.pointerId);
          // The header can collapse mid-drag. Keep the original tick positions
          // so the same pointer movement still selects the same message.
          dragging.current = { pointer: event.pointerId, first, last: index, top: bounds.top, pitch: bounds.height / count, count };
          setKeyboardIndex(null);
          setBrowseStart(first);
          preview(index, target);
          navigate(index);
        }}
        onPointerMove={(event) => {
          const drag = dragging.current;
          if (!drag || drag.pointer !== event.pointerId) return;
          const index = Math.max(drag.first, Math.min(drag.first + drag.count - 1, drag.first + Math.floor((event.clientY - drag.top) / drag.pitch)));
          if (index === drag.last) return;
          drag.last = index;
          const tick = ticks.current?.querySelector<HTMLElement>(`[data-chat-index-target="${index}"]`);
          if (tick) preview(index, tick);
          navigate(index);
        }}
        onPointerUp={() => { dragging.current = null; setBrowseStart(null); setHover(null); }}
        onPointerCancel={() => { dragging.current = null; setBrowseStart(null); setHover(null); }}
        onLostPointerCapture={() => { dragging.current = null; setBrowseStart(null); }}
      >
        <div ref={ticks} className="w-full shrink-0">
          {entries.slice(first, first + count).map((entry, offset) => {
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
                className="group flex w-full cursor-pointer items-center justify-end pr-1 outline-none"
                style={{ height: layout.pitch }}
                // Previews follow focus and a held pointer, never a passing one.
                onFocus={(event) => preview(index, event.currentTarget)}
                onClick={(event) => { if (event.detail === 0) navigate(index); }}
              >
                <span aria-hidden="true" className={cn(
                  "h-0.5 origin-right rounded-full transition-[transform,background-color] duration-150 group-focus-visible:scale-x-150 group-active:scale-x-150 motion-reduce:transition-none",
                  entry.role === "user" ? "w-2" : "w-4",
                  selected ? "bg-primary" : "bg-text-tertiary/80 group-focus-visible:bg-foreground group-active:bg-foreground",
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
