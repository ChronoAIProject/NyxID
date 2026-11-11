import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { serviceCredentialStatus } from "@/lib/service-credential-status";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  useLinkProviderService,
  useProviderServices,
  useServices,
} from "@/hooks/use-services";
import type { DownstreamService, ProviderConfig } from "@/types/api";
import { CreateServiceDialog } from "@/components/services/create-service-dialog";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ApiError } from "@/lib/api-client";
import { lanePriceLabel } from "@/schemas/platform-keys";

function laneSummary(service: DownstreamService) {
  const billing = service.billing;
  const lane = (
    name: string,
    price: NonNullable<typeof billing>["byok_pricing"],
  ) => `${name}: ${lanePriceLabel(price)}`;
  return billing?.byok_pricing || billing?.platform_key_pricing
    ? `${lane("Own key", billing.byok_pricing)} · ${lane("Platform key", billing.platform_key_pricing)}`
    : billing?.platform_billable
      ? "Legacy platform billing applies"
      : "No platform charge";
}

export function ProviderServices({
  provider,
}: {
  readonly provider: ProviderConfig;
}) {
  useUiTranslation();
  const { data: linked, isLoading, error } = useProviderServices(provider.id);
  const { data: services } = useServices();
  const link = useLinkProviderService();
  const [selected, setSelected] = useState("");
  const [createOpen, setCreateOpen] = useState(false);
  const supportsServices = ["api_key", "oauth2", "device_code"].includes(
    provider.provider_type,
  );
  const candidates =
    services?.filter(
      (service) =>
        service.service_type === "http" &&
        service.auth_method !== "oidc" &&
        !service.provider_config_id &&
        !linked?.some((row) => row.id === service.id),
    ) ?? [];
  return (
    <section
      className="space-y-4 rounded-lg border p-5"
      aria-label={translateUi("Linked services")}
    >
      <div>
        <h3 className="font-semibold">{translateUi("Linked services")}</h3>
        <p className="text-13 text-muted-foreground">{translateUi("Choose the service to configure. Shared credentials, access, endpoint rules, and prices belong to each service. A provider can back multiple services.")}</p>
      </div>
      {isLoading && <p>{translateUi("Loading services…")}</p>}
      {error && <p role="alert">{translateUi("Could not load linked services.")}</p>}
      {linked?.length === 0 && (
        <p className="text-14 text-muted-foreground">{translateUi("No linked services. Link an existing service or create one to configure access.")}</p>
      )}
      {linked?.map((service) => (
        <div key={service.id} className="space-y-2 rounded-md border p-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <span className="font-medium">{service.name}</span>{" "}
              <code className="text-14">{service.slug}</code>
            </div>
            <Button variant="outline" asChild>
              <Link
                to="/services/$serviceId/edit"
                params={{ serviceId: service.id }}
              >{translateUi("Configure service")}</Link>
            </Button>
          </div>
          {!service.provider_config_id && (
            <div className="flex items-center gap-2">
              <p className="text-13 text-muted-foreground">{translateUi("Legacy provider requirement; primary provider link is not set.")}</p>
              <Button
                type="button"
                variant="outline"
                disabled={!provider.is_active || link.isPending}
                onClick={() =>
                  link.mutate({
                    providerId: provider.id,
                    serviceId: service.id,
                  })
                }
              >{translateUi("Set primary provider")}</Button>
            </div>
          )}
          <p className="text-13">{translateUi("Shared credential: ")}{serviceCredentialStatus(service)}{translateUi(" · Platform key:")}{" "}
            {service.platform_key?.enabled || service.legacy_public_master
              ? translateUi("enabled")
              : translateUi("disabled")}{" "}{translateUi("· Audience:")}{" "}
            {service.platform_key?.audience === "public" ||
            service.legacy_public_master
              ? translateUi("all authenticated users")
              : translateUi("{{value1}} selected owners", { value1: service.platform_key?.allowed_owner_ids.length ?? 0 })}
          </p>
          <p className="text-13">{laneSummary(service)}</p>
        </div>
      ))}
      {supportsServices && !provider.is_active && (
        <p className="text-14 text-muted-foreground">{translateUi("Enable this provider before linking or creating services.")}</p>
      )}
      {supportsServices && (
        <>
          <div className="flex flex-wrap items-end gap-2">
            <div className="min-w-56 flex-1 space-y-1">
              <Label htmlFor="provider-service-choice">{translateUi("Existing service")}</Label>
              <Select value={selected} onValueChange={setSelected}>
                <SelectTrigger id="provider-service-choice">
                  <SelectValue placeholder={translateUi("Select a service")} />
                </SelectTrigger>
                <SelectContent>
                  {candidates.map((service) => (
                    <SelectItem key={service.id} value={service.id}>
                      {service.name} ({service.slug})
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <Button
              variant="outline"
              disabled={
                !provider.is_active ||
                !selected ||
                link.isPending ||
                Boolean(error)
              }
              onClick={() =>
                link.mutate(
                  { providerId: provider.id, serviceId: selected },
                  { onSuccess: () => setSelected("") },
                )
              }
            >{translateUi("Link service")}</Button>
            <Button
              variant="outline"
              disabled={!provider.is_active}
              onClick={() => setCreateOpen(true)}
            >{translateUi("Create linked service")}</Button>
          </div>
          {link.error && (
            <p role="alert" className="text-14 text-destructive">
              {link.error instanceof ApiError
                ? link.error.message
                : translateUi("Could not link service")}
            </p>
          )}
          <CreateServiceDialog
            open={createOpen}
            onOpenChange={setCreateOpen}
            provider={provider}
          />
        </>
      )}
    </section>
  );
}
