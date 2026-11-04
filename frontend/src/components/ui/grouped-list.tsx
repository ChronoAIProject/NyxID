import { TEXT_STYLES } from "@/lib/typography";
import { DrawerSectionInfoContext, drawerSectionInfo } from "@/lib/drawer-section-info";
import { SectionInfo } from "./section-info";
import { useDrawerSearchTarget, type DrawerSearchMetadata } from "@/lib/drawer-search";
import {
  createContext,
  cloneElement,
  isValidElement,
  useContext,
  useId,
  useState,
  type ComponentProps,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { Slot } from "@radix-ui/react-slot";
import { ChevronRight } from "lucide-react";
import { Input } from "./input";
import { Textarea } from "./textarea";
import { SelectTrigger } from "./select";
import { cn } from "@/lib/utils";

const FooterContext = createContext<HTMLDivElement | null>(null);

export function GroupedFooter({ children }: { readonly children: ReactNode }) {
  const container = useContext(FooterContext);
  return container ? createPortal(children, container) : null;
}

/** A single inset card. Compose groups as siblings, with explanatory text outside the card. */
export function GroupedList({
  title,
  info,
  search,
  footer,
  children,
  className,
  ...props
}: ComponentProps<"section"> & {
  readonly info?: string;
  readonly search?: DrawerSearchMetadata;
  readonly title?: string;
  readonly footer?: ReactNode;
}) {
  const drawer = useContext(DrawerSectionInfoContext);
  const help = info ?? (drawer && title ? drawerSectionInfo(title) : undefined);
  const id = useId();
  const metadata = search ?? (drawer && title ? { id: `section-${id}`, title } : undefined);
  const searchTarget = useDrawerSearchTarget(metadata ? { ...metadata, keywords: `${metadata.keywords ?? ""} ${help ?? ""}` } : undefined);
  const [footerContainer, setFooterContainer] = useState<HTMLDivElement | null>(
    null,
  );
  return (
    <FooterContext.Provider value={footerContainer}>
      <section
        ref={searchTarget}
        data-drawer-search-target={metadata?.id}
        aria-labelledby={title ? id : undefined}
        className={cn("space-y-2", className)}
        {...props}
      >
        {title && (
          <h3
            aria-label={title}
            id={id}
            className={`flex items-center gap-1 break-words px-4 ${TEXT_STYLES.groupHeading} font-medium text-text-tertiary`}
          >
            <span>{title}</span>
            {help && <SectionInfo title={title} >{help}</SectionInfo>}
          </h3>
        )}
        {children != null && (
          <div
            data-grouped-list=""
            className="overflow-hidden rounded-xl border border-border/50 bg-card empty:hidden"
          >
            {children}
          </div>
        )}
        <div
          ref={setFooterContainer}
          data-grouped-footer=""
          className="space-y-1 break-words px-4 text-13 font-normal leading-relaxed text-text-tertiary empty:hidden"
        >
          {typeof footer === "string" || typeof footer === "number" ? <p>{footer}</p> : footer}
        </div>
      </section>
    </FooterContext.Provider>
  );
}

/** Use asChild for a label, button, link or form item; interactive rows keep native semantics. */
export function GroupedRow({
  asChild,
  interactive = false,
  title,
  search,
  description,
  leading,
  accessory,
  children,
  className,
  ...props
}: Omit<ComponentProps<"div">, "title"> & {
  readonly asChild?: boolean;
  readonly interactive?: boolean;
  readonly search?: DrawerSearchMetadata;
  readonly title?: ReactNode;
  readonly description?: ReactNode;
  readonly leading?: ReactNode;
  readonly accessory?: ReactNode;
}) {
  const searchTarget = useDrawerSearchTarget(search);
  const Component = asChild ? Slot : "div";
  const child = asChild && isValidElement<{ className?: string }>(children) ? children : null;
  return (
    <Component
      ref={searchTarget}
      data-drawer-search-target={search?.id}
      data-grouped-row=""
      className={cn(
        `relative not-first:before:absolute not-first:before:left-4 not-first:before:right-0 not-first:before:top-0 not-first:before:h-px not-first:before:bg-border/50 flex min-h-11 w-full flex-wrap items-center gap-x-3 gap-y-2 px-4 py-2.5 text-left ${TEXT_STYLES.fieldLabel} text-foreground`,
        leading && "not-first:before:left-11",
        interactive &&
          "cursor-pointer transition-colors hover:bg-overlay active:bg-overlay-strong focus-visible:outline-none disabled:cursor-not-allowed disabled:text-text-tertiary disabled:hover:bg-transparent",
        className,
        child?.props.className,
      )}
      {...props}
    >
      {asChild
        ? child ? cloneElement(child, { className: undefined }) : children
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
                  <div className="mt-0.5 text-13 font-normal leading-relaxed text-text-tertiary">
                    {description}
                  </div>
                )}
              </div>
              {accessory && (
                <div className="ml-auto flex max-w-full flex-wrap items-center justify-end gap-2 text-14 font-normal text-text-tertiary">
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
  accessory,
  search,
  className,
  ...props
}: ComponentProps<"button"> & { readonly accessory?: ReactNode; readonly search?: DrawerSearchMetadata }) {
  return (
    <GroupedRow asChild interactive className={className} search={search}>
      <button type="button" {...props}>
        <span className="min-w-0 flex-1 break-words">{children}</span>
        {accessory}
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
        "h-auto rounded-none border-0 font-normal px-0 py-1 shadow-none focus-visible:ring-0",
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
        "rounded-none border-0 bg-transparent px-0 py-1 text-14 font-normal shadow-none focus-visible:ring-0",
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
      data-slot="grouped-select"
      className={cn(
        "ml-auto h-auto min-h-8 w-auto max-w-full gap-2 font-normal rounded-md border-0 bg-transparent px-0 py-1 shadow-none focus-visible:ring-0 [&>span]:line-clamp-none [&>span]:whitespace-normal [&>span]:break-words [&>span]:text-left",
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
        "sticky bottom-0 z-10 flex shrink-0 items-center justify-end gap-2 border-t border-border/50 bg-background px-4 py-2",
        className,
      )}
      {...props}
    />
  );
}
