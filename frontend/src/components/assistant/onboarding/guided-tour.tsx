import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Button } from "@/components/ui/button";
import { capture } from "@/lib/telemetry";
import { ASSISTANT_TOUR, type TourStep } from "./guided-tour-steps";

const PAD = 6;
const GAP = 12;
const CARD_WIDTH = 300;

function visibleTarget(name: string): HTMLElement | undefined {
  return [...document.querySelectorAll<HTMLElement>(`[data-tour="${name}"]`)].find((element) => {
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0 && getComputedStyle(element).visibility !== "hidden";
  });
}

interface Placement {
  readonly hole: { top: number; left: number; width: number; height: number };
  readonly card: { top: number; left: number };
}

function place(element: HTMLElement, cardHeight: number): Placement {
  const box = element.getBoundingClientRect();
  // Long lists (the sidebar) are spotlit from their top, not end to end.
  const height = Math.min(box.height, window.innerHeight * 0.4);
  const rect = { top: box.top, left: box.left, width: box.width, height };
  const hole = {
    top: rect.top - PAD,
    left: rect.left - PAD,
    width: rect.width + PAD * 2,
    height: rect.height + PAD * 2,
  };
  const width = Math.min(CARD_WIDTH, window.innerWidth - 2 * GAP);
  const clampLeft = (left: number) => Math.min(Math.max(GAP, left), window.innerWidth - width - GAP);
  const clampTop = (top: number) => Math.min(Math.max(GAP, top), window.innerHeight - cardHeight - GAP);
  // Beside narrow targets (the sidebar), otherwise above or below.
  if (hole.left + hole.width + GAP + width <= window.innerWidth && rect.width < window.innerWidth / 3) {
    return { hole, card: { left: hole.left + hole.width + GAP, top: clampTop(rect.top) } };
  }
  const below = hole.top + hole.height + GAP;
  const top = below + cardHeight <= window.innerHeight - GAP ? below : hole.top - GAP - cardHeight;
  return { hole, card: { left: clampLeft(rect.left), top: clampTop(top) } };
}

/**
 * A spotlight tour over the live screen. Steps whose target is not on screen
 * (for example the sidebar inside a closed mobile drawer) are skipped.
 */
export function GuidedTour({
  steps = ASSISTANT_TOUR,
  onClose,
}: {
  readonly steps?: readonly TourStep[];
  readonly onClose: () => void;
}) {
  // The tour opens over an already-rendered screen; read its targets once.
  const [available] = useState(() => steps.filter((step) => visibleTarget(step.target)));
  const [index, setIndex] = useState(0);
  const [placement, setPlacement] = useState<Placement>();
  const card = useRef<HTMLDivElement>(null);
  const step = available[index];

  const startedAt = useRef(0);
  const finish = useCallback(
    (completed: boolean) => {
      void capture(
        completed
          ? {
              name: "ui.dialog_step_completed",
              props: { dialog_id: "nyxbot_tour", step: index, total_steps: available.length },
            }
          : {
              name: "ui.dialog_abandoned",
              props: { dialog_id: "nyxbot_tour", final_step: index, duration_ms: Date.now() - startedAt.current },
            },
      );
      onClose();
    },
    [available.length, index, onClose],
  );

  useEffect(() => {
    startedAt.current = Date.now();
    void capture({ name: "ui.dialog_opened", props: { dialog_id: "nyxbot_tour", entry_point: "intro" } });
  }, []);

  useEffect(() => {
    if (!available.length) onClose();
  }, [available.length, onClose]);

  useLayoutEffect(() => {
    if (!step) return;
    const element = visibleTarget(step.target);
    if (!element) return;
    element.scrollIntoView({ block: "nearest", inline: "nearest" });
    let frame = 0;
    const update = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const target = visibleTarget(step.target);
        if (target) setPlacement(place(target, card.current?.offsetHeight ?? 160));
      });
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [step]);

  useEffect(() => {
    card.current?.focus({ preventScroll: true });
  }, [index]);

  // Modal: the app behind the tour is inert, so focus and typing stay here.
  useEffect(() => {
    const app = document.getElementById("root");
    if (!app || app.contains(card.current)) return;
    app.inert = true;
    return () => {
      app.inert = false;
    };
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target;
      if (target instanceof HTMLElement && (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))) {
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        finish(false);
      } else if (event.key === "ArrowRight") {
        setIndex((value) => Math.min(value + 1, available.length - 1));
      } else if (event.key === "ArrowLeft") {
        setIndex((value) => Math.max(value - 1, 0));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [available.length, finish]);

  if (!step) return null;
  const last = index === available.length - 1;
  return createPortal(
    // Above banners and toasts: the tour is the only thing the user is doing.
    <div className="fixed inset-0 z-[1000]" data-guided-tour="">
      {/* Clicks outside the spotlight end the tour. */}
      <button
        type="button"
        aria-label="End tour"
        tabIndex={-1}
        className="absolute inset-0 cursor-default"
        onClick={() => finish(false)}
      />
      {placement ? (
        <>
          {/* Dim everything but the spotlight (an SVG mask stays reliable for
              large holes, unlike a huge box-shadow). */}
          <svg aria-hidden="true" className="pointer-events-none absolute inset-0 h-full w-full">
            <defs>
              <mask id="guided-tour-mask">
                <rect width="100%" height="100%" fill="white" />
                <rect
                  x={placement.hole.left}
                  y={placement.hole.top}
                  width={placement.hole.width}
                  height={placement.hole.height}
                  rx={12}
                  fill="black"
                />
              </mask>
            </defs>
            <rect width="100%" height="100%" fill="rgb(0 0 0 / 0.6)" mask="url(#guided-tour-mask)" />
          </svg>
          <div
            aria-hidden="true"
            className="pointer-events-none absolute rounded-xl ring-2 ring-input-focus transition-all duration-300 ease-out motion-reduce:transition-none"
            style={placement.hole}
          />
        </>
      ) : null}
      <div
        ref={card}
        role="dialog"
        aria-modal="true"
        aria-labelledby="guided-tour-title"
        aria-describedby="guided-tour-body"
        tabIndex={-1}
        className="absolute space-y-2 rounded-xl border border-border bg-popover p-4 shadow-xl outline-none transition-[top,left] duration-300 ease-out animate-in fade-in-0 zoom-in-95 motion-reduce:transition-none motion-reduce:animate-none"
        style={{
          width: Math.min(CARD_WIDTH, window.innerWidth - 2 * GAP),
          top: placement?.card.top ?? -9999,
          left: placement?.card.left ?? -9999,
        }}
      >
        <p className="text-10 font-semibold uppercase tracking-[1.5px] text-text-tertiary">
          {index + 1} of {available.length}
        </p>
        <h2 id="guided-tour-title" className="text-13 font-semibold text-foreground">
          {step.title}
        </h2>
        <p id="guided-tour-body" className="text-12 leading-relaxed text-muted-foreground">
          {step.body}
        </p>
        <div className="flex items-center gap-2 pt-1">
          <Button variant="ghost" size="sm" onClick={() => finish(false)}>
            End tour
          </Button>
          <div className="ml-auto flex gap-2">
            {index > 0 ? (
              <Button variant="outline" size="sm" onClick={() => setIndex(index - 1)}>
                Back
              </Button>
            ) : null}
            <Button
              variant="primary"
              size="sm"
              onClick={() => (last ? finish(true) : setIndex(index + 1))}
            >
              {last ? "Done" : "Next"}
            </Button>
          </div>
        </div>
      </div>
    </div>,
    document.body,
  );
}
