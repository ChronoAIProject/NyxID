import { useId, type ComponentProps, type ReactNode } from "react";
import { Slot } from "@radix-ui/react-slot";
import { ChevronRight } from "lucide-react";
import { Input } from "./input";
import { Textarea } from "./textarea";
import { SelectTrigger } from "./select";
import { cn } from "@/lib/utils";

/** A single inset card. Compose groups as siblings, with explanatory text outside the card. */
export function GroupedList({
  title,
  footer,
  children,
  className,
  ...props
}: ComponentProps<"section"> & {
  readonly title?: string;
  readonly footer?: ReactNode;
}) {
  const id = useId();
  return (
    <section
      aria-labelledby={title ? id : undefined}
      className={cn("space-y-2", className)}
      {...props}
    >
      {title && (
        <h3 id={id} className="px-4 text-12 font-medium text-text-tertiary">
          {title}
        </h3>
      )}
      <div
        data-grouped-list=""
        className="overflow-hidden rounded-xl border border-border/50 bg-card"
      >
        {children}
      </div>
      {footer && (
        <div className="space-y-1 px-4 text-11 leading-relaxed text-text-tertiary">
          {footer}
        </div>
      )}
    </section>
  );
}

/** Use asChild for a label, button, link or form item; interactive rows keep native semantics. */
export function GroupedRow({
  asChild,
  interactive = false,
  title,
  description,
  leading,
  accessory,
  children,
  className,
  ...props
}: Omit<ComponentProps<"div">, "title"> & {
  readonly asChild?: boolean;
  readonly interactive?: boolean;
  readonly title?: ReactNode;
  readonly description?: ReactNode;
  readonly leading?: ReactNode;
  readonly accessory?: ReactNode;
}) {
  const Component = asChild ? Slot : "div";
  return (
    <Component
      data-grouped-row=""
      className={cn(
        "relative not-first:before:absolute not-first:before:left-4 not-first:before:right-0 not-first:before:top-0 not-first:before:h-px not-first:before:bg-border/50 flex min-h-11 w-full flex-wrap items-center gap-x-3 gap-y-2 px-4 py-2.5 text-left text-12",
        leading && "not-first:before:left-11",
        interactive &&
          "cursor-pointer transition-colors hover:bg-overlay active:bg-overlay-strong focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring disabled:cursor-not-allowed disabled:text-text-tertiary disabled:hover:bg-transparent",
        className,
      )}
      {...props}
    >
      {asChild
        ? children
        : (children ?? (
            <>
              {leading && (
                <span className="shrink-0 text-text-tertiary [&>svg]:size-[1.125em]">
                  {leading}
                </span>
              )}
              <div className="min-w-0 flex-1 basis-32 break-words">
                <div className="font-medium text-foreground">{title}</div>
                {description && (
                  <div className="mt-0.5 text-11 leading-relaxed text-text-tertiary">
                    {description}
                  </div>
                )}
              </div>
              {accessory && (
                <div className="flex max-w-full flex-wrap items-center gap-2 text-12 text-text-tertiary">
                  {accessory}
                </div>
              )}
            </>
          ))}
    </Component>
  );
}

export function GroupedDisclosure({
  children,
  className,
  ...props
}: ComponentProps<"button">) {
  return (
    <GroupedRow asChild interactive className={className}>
      <button type="button" {...props}>
        <span className="min-w-0 flex-1 break-words">{children}</span>
        <ChevronRight
          aria-hidden
          className="size-[1.125em] shrink-0 text-text-tertiary"
        />
      </button>
    </GroupedRow>
  );
}

export function GroupedInput({
  className,
  ...props
}: ComponentProps<typeof Input>) {
  return (
    <Input
      className={cn(
        "h-auto rounded-none border-0 px-0 py-1 shadow-none focus-visible:ring-2 focus-visible:ring-ring",
        className,
      )}
      {...props}
    />
  );
}

export function GroupedTextarea({
  className,
  ...props
}: ComponentProps<typeof Textarea>) {
  return (
    <Textarea
      className={cn(
        "rounded-none border-0 bg-transparent px-0 py-1 text-12 shadow-none focus-visible:ring-2 focus-visible:ring-ring",
        className,
      )}
      {...props}
    />
  );
}

export function GroupedSelectTrigger({
  className,
  ...props
}: ComponentProps<typeof SelectTrigger>) {
  return (
    <SelectTrigger
      className={cn(
        "h-auto min-h-8 w-auto max-w-full gap-2 rounded-md border-0 bg-transparent px-0 py-1 shadow-none [&>span]:line-clamp-none [&>span]:whitespace-normal [&>span]:break-words [&>span]:text-left",
        className,
      )}
      {...props}
    />
  );
}

export function GroupedSaveBar({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "sticky bottom-0 flex flex-wrap justify-end gap-2 bg-background py-3",
        className,
      )}
      {...props}
    />
  );
}
