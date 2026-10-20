import { useAuthStore } from "@/stores/auth-store";
import type { ComponentProps } from "react";
import { AsyncOptionSelect } from "@/components/shared/async-option-select";

export function ServiceAccountScopePicker({
  value,
  onChange,
  ownerId,
  serviceAccountId,
  ...props
}: {
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly ownerId: string;
  readonly serviceAccountId?: string;
} & Pick<
  ComponentProps<typeof AsyncOptionSelect>,
  "id" | "onBlur" | "disabled" | "aria-describedby" | "aria-invalid" | "ref"
>) {
  const isAdmin = useAuthStore((state) => state.user?.is_admin ?? false);
  return (
    <div className="space-y-2">
      <p className="text-xs text-muted-foreground">
        Choose scope segments or type a complete scope. Select a pill to edit
        it.
      </p>
      <AsyncOptionSelect
        {...props}
        optionSet="service-scope"
        label="Allowed scopes"
        allowCustom
        delimiter=":"
        context={{
          kind: "service-scope",
          principal_type: "service_account",
          owner_id: ownerId,
          service_account_id: serviceAccountId,
        }}
        value={value.split(/\s+/).filter(Boolean)}
        onChange={(values) => onChange(values.join(" "))}
      />
      <p className="text-xs text-muted-foreground">
        {isAdmin
          ? "Saving catalog:skills:read grants catalog and key metadata reads across all current and future catalog services. Add catalog:skills:write to amend skill recommendations. These accounts are managed by platform administrators."
          : "Platform catalog scopes must be granted by a platform administrator."}
      </p>
    </div>
  );
}
