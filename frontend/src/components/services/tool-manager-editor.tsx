import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { useCatalogToolMutation, useToolIcon } from "@/hooks/use-catalog-admin";
import { CatalogToolMetadata } from "./catalog-tool-metadata";
import { EndpointList } from "@/components/dashboard/endpoint-list";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { PageHeader } from "@/components/shared/page-header";
import type { DownstreamService } from "@/types/api";
import { toast } from "sonner";
import {
  toolManagerDestinationSchema as destinationSchema,
  toolManagerCredentialSchema as schema,
} from "@/schemas/tools";
import { ServiceIcon } from "@/components/service-icon";

export function ToolManagerEditor({ service }: { service: DownstreamService }) {
  const mutation = useCatalogToolMutation();
  const icon = useToolIcon();
  const form = useAppForm<z.infer<typeof schema>>({
    resolver: zodResolver(schema),
    defaultValues: { credential: "" },
  });
  const destination = useAppForm<z.infer<typeof destinationSchema>>({
    resolver: zodResolver(destinationSchema),
    defaultValues: { base_url: service.base_url },
  });
  const frozen =
    service.tools_credential_frozen || service.credential_configured;
  const canSetCredential =
    service.auth_method !== "none" &&
    !service.credential_configured &&
    !service.tools_credential_frozen;
  return (
    <div className="space-y-6">
      <PageHeader
        title={service.name}
        description="Prepare and validate this Tool. An Admin approves audience, prices and publication."
      />
      <CatalogToolMetadata service={service} />
      <form
        className="space-y-3 rounded-xl border border-border/50 bg-card p-4"
        onSubmit={destination.handleSubmit(async (values) => {
          try {
            await mutation.mutateAsync({ serviceId: service.id, body: values });
            destination.reset(values);
            toast.success("Destination updated");
          } catch {
            toast.error(
              "Destination update failed; reread the Tool before retrying",
            );
          }
        })}
      >
        <label className="text-12">
          API base URL
          <Input disabled={!!frozen} {...destination.register("base_url")} />
        </label>
        <p className="text-11 text-text-tertiary">
          {frozen
            ? "Destination frozen. Changes require an Admin."
            : "Review the vendor destination before setting the first credential."}
        </p>
        {!frozen && (
          <Button
            type="submit"
            disabled={
              !destination.formState.isDirty || !destination.formState.isValid
            }
            isLoading={mutation.isPending}
          >
            Save destination
          </Button>
        )}
      </form>
      <section className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
        <h2 className="text-15 font-semibold">Shared vendor credential</h2>
        <p className="text-12 text-muted-foreground">
          {service.auth_method === "none"
            ? "No credential required."
            : service.credential_configured
              ? "Configured. Replacement requires an Admin."
              : service.tools_credential_frozen
                ? "An Admin must restore the credential."
                : "Set the first credential once. This freezes the destination URL."}
        </p>
        {canSetCredential && (
          <form
            className="space-y-3"
            onSubmit={form.handleSubmit(async (values) => {
              try {
                await mutation.mutateAsync({
                  serviceId: service.id,
                  body: { credential: values.credential },
                });
                form.reset();
                toast.success("Credential configured");
              } catch {
                toast.error(
                  "Credential installation failed; reread the Tool before retrying",
                );
              }
            })}
          >
            <Input
              type="password"
              autoComplete="new-password"
              aria-label="Shared vendor credential"
              {...form.register("credential")}
            />
            <Button
              type="submit"
              variant="primary"
              disabled={!form.formState.isDirty || !form.formState.isValid}
              isLoading={mutation.isPending}
            >
              Set first credential
            </Button>
          </form>
        )}
      </section>
      <section className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
        <h2 className="text-15 font-semibold">Tool icon</h2>
        {service.icon_url && (
          <span role="img" aria-label="Current tool icon" className="inline-flex">
            <ServiceIcon iconUrl={service.icon_url} size="lg" />
          </span>
        )}
        <Input
          type="file"
          accept="image/svg+xml,.svg"
          aria-label="Upload Tool SVG"
          disabled={icon.isPending}
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file)
              icon.mutate(
                { serviceId: service.id, file },
                {
                  onSuccess: () => toast.success("Tool icon saved"),
                  onError: () =>
                    toast.error(
                      "Use a static UTF-8 SVG with a viewBox, up to 128 KiB",
                    ),
                },
              );
            event.target.value = "";
          }}
        />
      </section>
      <EndpointList
        serviceId={service.id}
        service={service}
        canWrite
        hasApiSpecUrl={!!service.openapi_spec_url}
      />
    </div>
  );
}
