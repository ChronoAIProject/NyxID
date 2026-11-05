import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ServiceIcon } from "@/components/service-icon";
import { ConnectionArc } from "@/components/shared/connection-arc";
import { cn } from "@/lib/utils";

/**
 * NyxID ↔ service identity row with the request title, as on the hosted
 * connector page. Shared so the CLI wizard's service step matches it.
 */
export function ConnectionIdentityHeading({
  serviceSlug,
  serviceName,
  iconUrl,
  title,
  subtitle,
  className,
}: {
  readonly serviceSlug: string;
  readonly serviceName: string;
  readonly iconUrl?: string | null;
  readonly title: string;
  readonly subtitle: string;
  readonly className?: string;
}) {
  return (
    <div className={cn("space-y-5 text-center", className)}>
      <div
        className="flex items-start justify-center"
        role="img"
        aria-label={`NyxID connects to ${serviceName}`}
      >
        <div className="flex w-20 flex-col items-center gap-2.5">
          <div className="connection-identity-icon flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
            <NyxidIcon className="size-9" alt="" />
          </div>
          <span className="type-secondary-emphasis">NyxID</span>
        </div>
        <ConnectionArc />
        <div className="flex w-20 flex-col items-center gap-2.5">
          <div className="connection-identity-icon flex size-18 items-center justify-center overflow-hidden rounded-full border border-border bg-background">
            <ServiceIcon
              slug={serviceSlug}
              iconUrl={iconUrl}
              size="xl"
              className="text-foreground"
            />
          </div>
          <span className="type-secondary-emphasis">{serviceName}</span>
        </div>
      </div>
      <div className="space-y-2">
        <h1 className="type-compact-page-title text-foreground sm:type-page-title">
          {title}
        </h1>
        <p className="text-14 text-muted-foreground">{subtitle}</p>
      </div>
    </div>
  );
}
