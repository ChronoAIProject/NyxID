import type { ComponentProps, ReactNode } from "react";
import { cn } from "@/lib/utils";

interface PageHeaderProps extends Omit<ComponentProps<"header">, "title"> {
  readonly title: string;
  readonly description?: string;
  readonly actions?: ReactNode;
  readonly leading?: ReactNode;
  readonly titleAs?: "h1" | "h2";
}

export function PageHeader({
  title,
  description,
  actions,
  leading,
  titleAs: Title = "h2",
  className,
  ...props
}: PageHeaderProps) {
  return (
    <header data-page-header="" className={cn("flex min-h-0 min-w-0 flex-col gap-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4", className)} {...props}>
      <div data-page-heading="" className="flex min-w-0 flex-col gap-1">
        <div className="flex items-center gap-3">
          {leading && <div className="shrink-0">{leading}</div>}
          <Title className="text-22 sm:text-28 font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
            {title}
          </Title>
        </div>
        {description && <p className="text-14 unthemed:text-12 text-muted-foreground">{description}</p>}
      </div>
      {actions && <div data-title-actions="" className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </header>
  );
}
