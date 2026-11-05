import { useEffect, useId, useRef, useState } from "react";
import { Info } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";

/** A single controlled surface supports hover, keyboard focus and touch taps. */
export function SectionInfo({ title, children }: { readonly title: string; readonly children: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const show = () => { clearTimeout(timer.current); setOpen(true); };
  const leave = () => { timer.current = setTimeout(() => setOpen(false), 120); };
  useEffect(() => () => clearTimeout(timer.current), []);
  return <Popover open={open} onOpenChange={setOpen}>
    <PopoverTrigger asChild>
      <button type="button" data-icon-button="" aria-label={`About ${title}`} aria-describedby={open ? id : undefined} className="inline-flex min-h-6 min-w-6 shrink-0 items-center justify-center rounded-full text-text-tertiary hover:bg-overlay focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" onMouseEnter={show} onMouseLeave={leave} onFocus={show} onBlur={leave} onClick={(event) => { event.preventDefault(); show(); }}>
        <Info aria-hidden className="size-[1em]" />
      </button>
    </PopoverTrigger>
    <PopoverContent id={id} role="tooltip" aria-label={`About ${title}`} className="w-70 max-w-[calc(100vw-2rem)] p-3 type-secondary" onMouseEnter={show} onMouseLeave={leave} onOpenAutoFocus={(event) => event.preventDefault()} onCloseAutoFocus={(event) => event.preventDefault()} onEscapeKeyDown={(event) => { event.stopPropagation(); setOpen(false); }}>{children}</PopoverContent>
  </Popover>;
}
