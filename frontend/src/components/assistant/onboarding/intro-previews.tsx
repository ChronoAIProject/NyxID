import { useState, type CSSProperties, type ReactNode } from "react";
import { ShieldCheck } from "lucide-react";
import { useMediaQuery } from "@/hooks/use-media-query";
import { ACCESS_MODE_DESCRIPTION, ACCESS_MODE_LABEL } from "@/lib/assistant/nyxbot-labels";
import { cn } from "@/lib/utils";
import type { NyxAgentAccessMode } from "@/schemas/assistant-nyxagent";

/*
 * Real assistant screens for the first-run intro: recorded replays of the
 * product, and a live render of an action card. Sample data only.
 */


/** Staged entrance; reduced motion shows everything at once. */
function stage(index: number): { className: string; style: CSSProperties } {
  return {
    className: "animate-in fade-in-0 slide-in-from-bottom-2 duration-500 motion-reduce:animate-none",
    style: { animationDelay: `${150 + index * 350}ms`, animationFillMode: "both" },
  };
}

function WindowBar({ title }: { readonly title: string }) {
  return (
    <div className="flex items-center gap-2 border-b border-border/50 bg-sidebar px-3 py-2">
      <span aria-hidden="true" className="flex gap-1">
        <span className="h-2 w-2 rounded-full bg-overlay-strong" />
        <span className="h-2 w-2 rounded-full bg-overlay-strong" />
        <span className="h-2 w-2 rounded-full bg-overlay-strong" />
      </span>
      <span className="truncate text-11 font-medium text-text-tertiary">{title}</span>
    </div>
  );
}

function ScreenFrame({
  title,
  children,
  className,
}: {
  readonly title: string;
  readonly children: ReactNode;
  readonly className?: string;
}) {
  return (
    <div
      aria-hidden="true"
      className={cn(
        "pointer-events-none select-none overflow-hidden rounded-xl border border-border bg-background shadow-sm",
        className,
      )}
    >
      <WindowBar title={title} />
      <div className="space-y-3 p-3">{children}</div>
    </div>
  );
}

/**
 * A recorded replay of the real assistant (dev mock mode, fixture data;
 * regenerate with `scripts/record-onboarding-clips.mjs`). Loops silently like
 * a GIF; with reduced motion it waits for the play control instead.
 */
export function ReplayClip({
  clip,
  title,
  label,
}: {
  readonly clip: "chief-of-staff" | "specialists";
  readonly title: string;
  /** What the replay shows, for assistive tech. */
  readonly label: string;
}) {
  const reduced = useMediaQuery("(prefers-reduced-motion: reduce)");
  return (
    <div className="overflow-hidden rounded-xl border border-border bg-background shadow-sm">
      <WindowBar title={title} />
      <video
        key={clip}
        src={`/onboarding/${clip}.mp4`}
        aria-label={label}
        className="block aspect-[1280/864] w-full bg-background"
        autoPlay={!reduced}
        controls={reduced}
        loop
        muted
        playsInline
        preload="auto"
      />
    </div>
  );
}

const MODES: readonly NyxAgentAccessMode[] = ["nyxmode", "ask_every_time", "autopilot"];

export function ControlPreview() {
  const [mode, setMode] = useState<NyxAgentAccessMode>("nyxmode");
  const card = stage(0);
  return (
    <div className="space-y-3">
      <ScreenFrame title="NyxBot">
        <div
          className={cn("rounded-lg border border-border bg-card p-3 shadow-sm", card.className)}
          style={card.style}
        >
          <div className="flex items-center justify-between gap-2">
            <p className="flex items-center gap-1.5 text-12 font-semibold text-foreground">
              <ShieldCheck className="size-3.5 text-warning" />
              NyxBot wants to send an email
            </p>
            <span className="rounded-md bg-warning/15 px-1.5 py-0.5 text-10 font-medium text-warning">
              Sensitive
            </span>
          </div>
          <p className="mt-1 text-11 text-muted-foreground">To dana@acme.com · Re: Contract redlines</p>
          <div className="mt-2.5 flex gap-2">
            <span className="rounded-lg border border-success/30 bg-success/15 px-3 py-1 text-11 font-medium text-success">
              Allow
            </span>
            <span className="rounded-lg border border-destructive/30 bg-destructive/15 px-3 py-1 text-11 font-medium text-destructive">
              Deny
            </span>
          </div>
        </div>
      </ScreenFrame>
      <div className="space-y-1.5">
        <div role="radiogroup" aria-label="Access modes" className="grid grid-cols-3 gap-1 rounded-lg border border-border bg-muted p-1">
          {MODES.map((value) => (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={mode === value}
              onClick={() => setMode(value)}
              className={cn(
                "rounded-md px-2 py-1 text-11 font-medium transition-colors motion-reduce:transition-none",
                mode === value
                  ? "bg-card text-foreground shadow-sm"
                  : "text-text-tertiary hover:text-foreground",
              )}
            >
              {ACCESS_MODE_LABEL[value]}
            </button>
          ))}
        </div>
        <p className="min-h-[2lh] text-11 leading-snug text-muted-foreground">
          {ACCESS_MODE_DESCRIPTION[mode]}
          {mode === "nyxmode" ? " This is the default." : ""}
        </p>
      </div>
    </div>
  );
}
