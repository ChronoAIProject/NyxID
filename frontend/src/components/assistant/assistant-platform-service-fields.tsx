import { useKeys } from "@/hooks/use-keys";
import { useApiKey } from "@/hooks/use-api-keys";
import { PlatformServiceScope } from "@/components/shared/platform-service-scope";

export function AssistantPlatformServiceFields({
  keyId,
  targetOrgId,
  selectedIds,
  allowAll,
  onIdsChange,
  onAllowAllChange,
  disabled,
}: {
  readonly keyId?: string;
  readonly targetOrgId?: string;
  readonly selectedIds?: readonly string[];
  readonly allowAll?: boolean;
  readonly onIdsChange: (ids: string[]) => void;
  readonly onAllowAllChange: (value: boolean) => void;
  readonly disabled: boolean;
}) {
  const services = useKeys();
  const key = useApiKey(keyId ?? "");
  if (keyId && !key.data)
    return (
      <p className="text-[12px] text-muted-foreground">
        {key.isError
          ? "Could not load service scope."
          : "Loading service scope…"}
      </p>
    );
  const orgId =
    targetOrgId ||
    (key.data?.credential_source?.type === "org"
      ? key.data.credential_source.org_id
      : undefined);
  const ids = selectedIds ?? key.data?.allowed_service_ids ?? [];
  return (
    <PlatformServiceScope
      services={(services.data ?? []).filter(
        (service) =>
          service.is_active &&
          (orgId
            ? service.credential_source?.type === "org" &&
              service.credential_source.org_id === orgId
            : service.credential_source?.type !== "org"),
      )}
      selectedIds={ids}
      allowAll={allowAll ?? key.data?.allow_auto_connected_services ?? false}
      orgOwned={Boolean(orgId)}
      disabled={disabled || services.isLoading || services.isError}
      onAllowAllChange={onAllowAllChange}
      onToggle={(id) =>
        onIdsChange(
          ids.includes(id) ? ids.filter((value) => value !== id) : [...ids, id],
        )
      }
    />
  );
}
