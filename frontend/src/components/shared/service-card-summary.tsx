import type { ComponentProps, ReactNode } from "react";
import { ServiceCardHeader } from "./service-card-header";
import { cn } from "@/lib/utils";

/** The same identity, description line box and reserved space in both catalogs. */
export function ServiceCardSummary({
  description,
  hideDescription = false,
  wrap,
  className,
  ...header
}: ComponentProps<typeof ServiceCardHeader> & {
  readonly description: ReactNode;
  readonly hideDescription?: boolean;
  readonly wrap?: (content: ReactNode) => ReactNode;
}) {
  const identity = <ServiceCardHeader {...header} />;
  const detail = (
    <p data-service-card-description="" title={typeof description === "string" ? description : undefined} className="type-body line-clamp-2 min-h-[2lh] break-words text-muted-foreground">
      {description}
    </p>
  );
  return (
    <div data-service-card-summary="" className={cn("flex min-w-0 flex-col gap-2", className)}>
      {wrap ? wrap(identity) : identity}
      {!hideDescription && (wrap ? wrap(detail) : detail)}
    </div>
  );
}
