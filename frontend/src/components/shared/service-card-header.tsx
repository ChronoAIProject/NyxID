import type { ComponentProps, ReactNode } from "react";
import { TEXT_STYLES } from "@/lib/typography";
import { cn } from "@/lib/utils";

/** Service identity is the same role in Studio inventory and the plugin catalog. */
export function ServiceCardHeader({
  name,
  icon,
  subtitle,
  subtitleTitle,
  titleAction,
  titleId,
  children,
  className,
}: {
  readonly name: string;
  readonly icon: ReactNode;
  readonly subtitle: ReactNode;
  readonly subtitleTitle?: string;
  readonly titleAction?: Pick<ComponentProps<"button">, "onClick" | "aria-expanded" | "aria-controls">;
  readonly titleId?: string;
  readonly children?: ReactNode;
  readonly className?: string;
}) {
  return (
    <div data-service-card-header="" className={cn("flex items-start gap-3", className)}>
      <span data-service-card-icon="" className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-border bg-background/50 type-card-title text-muted-foreground">
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <h3 id={titleId} title={name} data-service-card-title="" className={cn(TEXT_STYLES.cardTitle, "relative tracking-tight")}>
          <span aria-hidden={titleAction ? true : undefined} className={cn("block truncate", titleAction && "invisible")}>{name}</span>
          {titleAction && (
            <button
              type="button"
              {...titleAction}
              className="absolute inset-x-0 top-1/2 flex w-full max-w-full -translate-y-1/2 cursor-pointer items-center text-left hover:text-primary-text focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ring"
            >
              <span className="block w-full truncate">{name}</span>
            </button>
          )}
        </h3>
        <p data-service-card-subtitle="" title={subtitleTitle} className="mt-0.5 truncate type-secondary text-muted-foreground">{subtitle}</p>
      </div>
      {children}
    </div>
  );
}
