import { useId, useRef, useState } from "react";
import { RotateCcw } from "lucide-react";
import { Button } from "./button";
import { Label } from "./label";

/** Shared range content for grouped rows in either shell. */
export function RangeControl({
  label,
  description,
  saved,
  onChange,
  min,
  max,
  step = 1,
  formatValue,
  defaultValue,
  commitOnRelease = true,
}: {
  readonly label: string;
  readonly description?: string;
  readonly saved: number;
  readonly onChange: (value: number) => void;
  readonly min: number;
  readonly max: number;
  readonly step?: number;
  readonly formatValue: (value: number) => string;
  readonly defaultValue: number;
  readonly commitOnRelease?: boolean;
}) {
  const id = useId();
  const [draft, setDraft] = useState<number | null>(null);
  const pointer = useRef<number | null>(null);
  const value = draft ?? saved;
  function cancel() {
    pointer.current = null;
    setDraft(null);
  }
  return (
    <div data-range-control="" className="w-full min-w-0 space-y-1">
      <div data-range-heading="" className="flex items-center gap-3">
        <Label htmlFor={id} className="min-w-0 flex-1">
          {label}
        </Label>
        <output
          htmlFor={id}
          aria-live="off"
          className="shrink-0 font-mono text-13 text-muted-foreground"
        >
          {formatValue(value)}
        </output>
        <Button
          size="icon"
          variant="ghost"
          aria-label={`Reset ${label.toLowerCase()}`}
          title={`Reset to ${formatValue(defaultValue)}`}
          disabled={saved === defaultValue && draft === null}
          onClick={() => {
            cancel();
            onChange(defaultValue);
          }}
        >
          <RotateCcw aria-hidden="true" className="size-3" />
        </Button>
      </div>
      <input
        id={id}
        type="range"
        aria-label={label}
        aria-valuetext={formatValue(value)}
        aria-describedby={description ? `${id}-help` : undefined}
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => {
          if (pointer.current === null)
            onChange(event.currentTarget.valueAsNumber);
          else setDraft(event.currentTarget.valueAsNumber);
        }}
        onPointerDown={(event) => {
          if (
            !commitOnRelease ||
            event.button !== 0 ||
            pointer.current !== null
          )
            return;
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
        className="block h-6 w-full min-w-0 cursor-pointer accent-primary focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
      />
      {description && (
        <p id={`${id}-help`} className="text-13 text-text-tertiary">
          {description}
        </p>
      )}
    </div>
  );
}
