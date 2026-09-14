import { useId } from "react";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";

/** The durable grant implies platform rows without changing explicit selections. */
export function PlatformServiceScope({
  services,
  selectedIds,
  allowAll = false,
  onAllowAllChange,
  onToggle,
  orgOwned = false,
  disabled = false,
}: {
  readonly services: readonly {
    readonly id: string;
    readonly name?: string;
    readonly label?: string;
    readonly slug?: string;
    readonly auto_connected?: boolean;
    /** Foreign-owner platform rows remain explicitly selectable. */
    readonly platform_grant_eligible?: boolean;
  }[];
  readonly selectedIds: readonly string[];
  readonly allowAll?: boolean;
  readonly onAllowAllChange: (value: boolean) => void;
  readonly onToggle: (id: string) => void;
  readonly orgOwned?: boolean;
  readonly disabled?: boolean;
}) {
  const prefix = useId();
  if (orgOwned && !services.some((service) => service.auto_connected)) {
    return (
      <p className="text-[12px] text-muted-foreground">
        This org-owned key cannot use platform services from your personal account.
      </p>
    );
  }
  return (
    <section
      aria-label="Auto-connected platform services"
      className="space-y-2 border-t border-border/50 pt-3"
    >
      <p className="text-[12px] font-medium">
        Auto-connected platform services
      </p>
      <Label className="flex items-start gap-2 text-[12px]">
        <Checkbox
          checked={allowAll}
          disabled={disabled}
          onCheckedChange={(value) => onAllowAllChange(value === true)}
        />
        Allow all auto-connected platform services (includes ones added later)
      </Label>
      {services
        .filter((service) => service.auto_connected)
        .map((service) => {
          const implied = allowAll && service.platform_grant_eligible !== false;
          return (
            <Label
              key={service.id}
              htmlFor={`${prefix}-${service.id}`}
              className={`flex items-center gap-2 text-[12px] ${implied ? "text-muted-foreground" : ""}`}
            >
              <Checkbox
                id={`${prefix}-${service.id}`}
                checked={implied || selectedIds.includes(service.id)}
                disabled={disabled || implied}
                onCheckedChange={() => onToggle(service.id)}
              />
              {service.label || service.name || service.slug || service.id}
              {service.platform_grant_eligible === false &&
                " (Organization; select individually)"}
            </Label>
          );
        })}
    </section>
  );
}
