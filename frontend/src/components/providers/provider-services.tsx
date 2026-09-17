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

function laneSummary(service: DownstreamService) {
  const billing = service.billing;
  const lane = (
    name: string,
    price: NonNullable<typeof billing>["byok_pricing"],
  ) =>
    `${name}: ${price ? `${price.credits_per_unit} credits/${price.metric} (${price.sync_status ?? "pending"})` : "free"}`;
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
      aria-label="Linked services"
    >
      <div>
        <h3 className="font-semibold">Linked services</h3>
        <p className="text-xs text-muted-foreground">
          Choose the service to configure. Shared credentials, access, endpoint
          rules, and prices belong to each service. A provider can back multiple
          services.
        </p>
      </div>
      {isLoading && <p>Loading services…</p>}
      {error && <p role="alert">Could not load linked services.</p>}
      {linked?.length === 0 && (
        <p className="text-sm text-muted-foreground">
          No linked services. Link an existing service or create one to
          configure access.
        </p>
      )}
      {linked?.map((service) => (
        <div key={service.id} className="space-y-2 rounded-md border p-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <span className="font-medium">{service.name}</span>{" "}
              <code className="text-xs">{service.slug}</code>
            </div>
            <Button variant="outline" asChild>
              <Link
                to="/services/$serviceId/edit"
                params={{ serviceId: service.id }}
              >
                Configure service
              </Link>
            </Button>
          </div>
          {!service.provider_config_id && (
            <div className="flex items-center gap-2">
              <p className="text-xs text-muted-foreground">
                Legacy provider requirement; primary provider link is not set.
              </p>
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
              >
                Set primary provider
              </Button>
            </div>
          )}
          <p className="text-xs">
            Shared credential: {serviceCredentialStatus(service)} · Platform
            key:{" "}
            {service.platform_key?.enabled || service.legacy_public_master
              ? "enabled"
              : "disabled"}{" "}
            · Audience:{" "}
            {service.platform_key?.audience === "public" ||
            service.legacy_public_master
              ? "all authenticated users"
              : `${service.platform_key?.allowed_owner_ids.length ?? 0} selected owners`}
          </p>
          <p className="text-xs">{laneSummary(service)}</p>
        </div>
      ))}
      {supportsServices && !provider.is_active && (
        <p className="text-sm text-muted-foreground">
          Enable this provider before linking or creating services.
        </p>
      )}
      {supportsServices && (
        <>
          <div className="flex flex-wrap items-end gap-2">
            <div className="min-w-56 flex-1 space-y-1">
              <Label htmlFor="provider-service-choice">Existing service</Label>
              <Select value={selected} onValueChange={setSelected}>
                <SelectTrigger id="provider-service-choice">
                  <SelectValue placeholder="Select a service" />
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
            >
              Link service
            </Button>
            <Button
              variant="outline"
              disabled={!provider.is_active}
              onClick={() => setCreateOpen(true)}
            >
              Create linked service
            </Button>
          </div>
          {link.error && (
            <p role="alert" className="text-sm text-destructive">
              {link.error instanceof ApiError
                ? link.error.message
                : "Could not link service"}
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
