import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useState } from "react";
import { cn } from "@/lib/utils";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ServiceOwnerAvatar } from "./service-owner-avatar";

export interface ServiceAvatarItem {
  readonly id: string;
  readonly type: "personal" | "org" | "platform";
  readonly name: string;
  readonly avatarUrl?: string | null;
  readonly detail?: string;
  readonly description?: string;
  readonly actionLabel?: string;
  readonly onSelect?: () => void;
}

export function ServiceAvatarStack({
  items,
  label,
}: {
  readonly items: readonly ServiceAvatarItem[];
  readonly label: string;
}) {
  const { t } = useUiTranslation();
  const [showAll, setShowAll] = useState(false);
  const visible = showAll ? items : items.slice(0, 4);
  return (
    <TooltipProvider delayDuration={100} disableHoverableContent>
      <div
        role="group"
        aria-label={label}
        className={cn("relative h-6 max-w-full shrink-0", showAll && "z-30")}
        style={{
          width:
            Math.min(items.length, 4) * 16 + 8 + (items.length > 4 ? 28 : 0),
        }}
        onBlurCapture={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget))
            setShowAll(false);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") setShowAll(false);
        }}
      >
        <div
          className={cn(
            "absolute inset-x-0 top-0 isolate flex items-start",
            showAll && "flex-wrap gap-y-1 rounded-xl bg-card p-1 shadow-sm",
          )}
        >
          {visible.map((item, index) => (
            <Tooltip key={item.id}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  aria-label={item.actionLabel ?? t("{{name}} · {{label}}", { name: item.name, label: label })}
                  onClick={item.onSelect}
                  className={cn(
                    "relative z-0 flex size-6 shrink-0 items-center justify-center rounded-full bg-card ring-2 ring-card hover:z-10 focus-visible:z-10 focus-visible:outline-2 focus-visible:outline-ring",
                    index > 0 && "-ml-2",
                  )}
                >
                  <ServiceOwnerAvatar {...item} className="size-6 bg-card" />
                </button>
              </TooltipTrigger>
              <TooltipContent
                side="top"
                sideOffset={6}
                collisionPadding={12}
                className="max-w-[min(20rem,calc(100vw-2rem))] break-words [overflow-wrap:anywhere] data-[state=delayed-open]:animate-none data-[state=instant-open]:animate-none data-[state=closed]:animate-none"
              >
                <p className="font-medium">{item.name}</p>
                {item.detail && <p className="mt-1">{item.detail}</p>}
                {item.description && (
                  <p className="mt-1 text-13 text-muted-foreground">
                    {item.description}
                  </p>
                )}
              </TooltipContent>
            </Tooltip>
          ))}
          {items.length > 4 && (
            <button
              type="button"
              aria-label={t("{{value}} · {{label}}", { value: showAll ? t("Collapse") : `Show all ${items.length} entries`, label: label })}
              aria-expanded={showAll}
              onClick={() => setShowAll((value) => !value)}
              className="relative ml-1 flex size-6 shrink-0 items-center justify-center rounded-full bg-muted type-group-label ring-2 ring-card focus-visible:outline-2 focus-visible:outline-ring"
            >
              {showAll ? "−" : t("+{{value}}", { value: items.length - 4 })}
            </button>
          )}
        </div>
      </div>
    </TooltipProvider>
  );
}
