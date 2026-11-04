import type { ReactNode } from "react";
import { TEXT_STYLES } from "@/lib/typography";
import { cn } from "@/lib/utils";

/** Service identity is the same role in Studio inventory and the plugin catalog. */
export function ServiceCardHeader({
  name,
  icon,
  subtitle,
  subtitleTitle,
  title,
  titleId,
  children,
  className,
}: {
  readonly name: string;
  readonly icon: ReactNode;
  readonly subtitle: ReactNode;
  readonly subtitleTitle?: string;
  readonly title?: ReactNode;
  readonly titleId?: string;
  readonly children?: ReactNode;
  readonly className?: string;
}) {
  return (
    <div data-service-card-header="" className={cn("flex items-start gap-3", className)}>
      <span data-service-card-icon="" className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-border bg-background/50 text-15 font-semibold text-muted-foreground">
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <h3 id={titleId} title={name} data-service-card-title="" className={cn(TEXT_STYLES.cardTitle, "truncate tracking-tight")}>
          {title ?? name}
        </h3>
        <p title={subtitleTitle} className="mt-0.5 truncate text-13 text-muted-foreground">{subtitle}</p>
      </div>
      {children}
    </div>
  );
}
