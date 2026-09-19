import type { ComponentProps } from "react";
import { AsyncOptionSelect } from "@/components/shared/async-option-select";

export function ServiceAccountScopePicker({ value, onChange, ownerId, serviceAccountId, ...props }: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly ownerId: string;
  readonly serviceAccountId?: string;
} & Pick<ComponentProps<typeof AsyncOptionSelect>, "id" | "onBlur" | "disabled" | "aria-describedby" | "aria-invalid" | "ref">) {
  return <div className="space-y-2"><p className="text-xs text-muted-foreground">Choose scope segments or type a complete scope. Select a pill to edit it.</p><AsyncOptionSelect {...props} optionSet="service-scope" label="Allowed scopes" allowCustom delimiter=":"
    context={{ principal_type: "service_account", owner_id: ownerId, service_account_id: serviceAccountId }}
    value={value.split(/\s+/).filter(Boolean)} onChange={(values) => onChange(values.join(" "))} /></div>;
}
