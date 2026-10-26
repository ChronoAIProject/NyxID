import { useRef, useState } from "react";
import { Columns2, RotateCcw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { CHAT_CONTENT_WIDTH, COMPOSER_MAX_HEIGHT, useThemeStore } from "@/stores/theme-store";

export function ChatWidthControl() {
  const width = useThemeStore((state) => state.chatContentWidth);
  const setWidth = useThemeStore((state) => state.setChatContentWidth);
  return <ChatDimensionControl label="Chat content width" saved={width} onChange={setWidth} range={CHAT_CONTENT_WIDTH} />;
}

export function ComposerHeightControl() {
  const height = useThemeStore((state) => state.composerMaxHeight);
  const setHeight = useThemeStore((state) => state.setComposerMaxHeight);
  return <ChatDimensionControl label="Maximum composer height" saved={height} onChange={setHeight} range={COMPOSER_MAX_HEIGHT} />;
}

function ChatDimensionControl({ label, saved, onChange, range }: {
  readonly label: string;
  readonly saved: number;
  readonly onChange: (value: number) => void;
  readonly range: { readonly min: number; readonly max: number; readonly default: number };
}) {
  const [draft, setDraft] = useState<number | null>(null);
  const pointer = useRef<number | null>(null);
  const value = draft ?? saved;
  function cancel() {
    pointer.current = null;
    setDraft(null);
  }
  return (
    <div className="flex w-full max-w-80 items-center gap-3">
      <input
        type="range"
        aria-label={label}
        aria-valuetext={`${value} pixels`}
        min={range.min}
        max={range.max}
        value={value}
        onChange={(event) => {
          if (pointer.current === null) onChange(event.currentTarget.valueAsNumber);
          else setDraft(event.currentTarget.valueAsNumber);
        }}
        onPointerDown={(event) => {
          if (event.button !== 0 || pointer.current !== null) return;
          pointer.current = event.pointerId;
          event.currentTarget.setPointerCapture(event.pointerId);
          setDraft(saved);
        }}
        onPointerUp={(event) => {
          if (pointer.current !== event.pointerId) return;
          onChange(event.currentTarget.valueAsNumber);
          cancel();
        }}
        onPointerCancel={cancel}
        onLostPointerCapture={cancel}
        className="h-6 min-w-0 flex-1 cursor-pointer accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      />
      <span className="shrink-0 font-mono text-11 text-muted-foreground">{value}px</span>
      <Button
        size="icon"
        variant="ghost"
        aria-label={`Reset ${label.toLowerCase()}`}
        disabled={saved === range.default}
        onClick={() => onChange(range.default)}
      >
        <RotateCcw aria-hidden="true" className="size-3" />
      </Button>
    </div>
  );
}

export function ChatLayoutMenu() {
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label="Chat layout"
          title="Chat layout"
          className="hidden h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline text-text-tertiary transition-colors hover:border-hairline-strong hover:text-muted-foreground focus-visible:outline-2 focus-visible:outline-ring md:flex"
        >
          <Columns2 aria-hidden="true" className="size-3.5" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="end" aria-label="Chat layout" className="space-y-3">
        <div>
          <p className="text-12 font-medium">Chat content width</p>
          <p className="mt-1 text-11 text-muted-foreground">Applies to all chats in this browser. You can also adjust this in Settings → Display.</p>
        </div>
        <ChatWidthControl />
      </PopoverContent>
    </Popover>
  );
}
