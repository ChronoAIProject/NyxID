import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useAuthStore } from "@/stores/auth-store";
import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { toolEditableMetadataSchema as schema } from "@/schemas/tools";
import { useToolTopics } from "@/hooks/use-tools";
import { useCatalogToolMutation } from "@/hooks/use-catalog-admin";
import type { DownstreamService } from "@/types/api";
export function CatalogToolMetadata({
  service,
}: {
  service: DownstreamService;
}) {
  const { t } = useUiTranslation();
  const admin = useAuthStore((state) => state.user?.is_admin) ?? false;
  const { data: topics = [] } = useToolTopics();
  const mutation = useCatalogToolMutation();
  const form = useAppForm<z.infer<typeof schema>>({
    resolver: zodResolver(schema),
    defaultValues: {
      name: service.name,
      description: service.description ?? "",
      offering_kind: service.offering_kind ?? "ai_service",
      topics: [...(service.topics ?? [])],
      supplier: service.supplier ?? "",
      openapi_spec_url: service.openapi_spec_url ?? "",
      homepage_url: service.homepage_url ?? "",
      repository_url: service.repository_url ?? "",
      issues_url: service.issues_url ?? "",
      examples_url: service.examples_url ?? "",
      auth_notes: service.auth_notes ?? "",
      known_limitations: service.known_limitations ?? "",
      required_permissions: (service.required_permissions ?? []).join(", "),
      capabilities: {
        supports_proxy_read: service.capabilities?.supports_proxy_read ?? false,
        supports_proxy_write:
          service.capabilities?.supports_proxy_write ?? false,
        supports_proxy_binary_upload:
          service.capabilities?.supports_proxy_binary_upload ?? false,
        supports_direct_downstream_auth:
          service.capabilities?.supports_direct_downstream_auth ?? false,
        supports_authoring_via_nyx:
          service.capabilities?.supports_authoring_via_nyx ?? false,
        supports_websocket: service.capabilities?.supports_websocket ?? false,
        supports_streaming: service.capabilities?.supports_streaming ?? false,
      },
    },
  });
  return (
    <form
      className="space-y-3"
      onSubmit={form.handleSubmit(async (values) => {
        try {
          await mutation.mutateAsync({
            serviceId: service.id,
            body: {
              name: values.name,
              description: values.description,
              topics: values.topics,
              homepage_url: values.homepage_url,
              repository_url: values.repository_url,
              issues_url: values.issues_url,
              examples_url: values.examples_url,
              auth_notes: values.auth_notes,
              known_limitations: values.known_limitations,
              ...(form.formState.dirtyFields.capabilities
                ? { capabilities: values.capabilities }
                : {}),
              required_permissions: values.required_permissions
                .split(",")
                .map((entry) => entry.trim())
                .filter(Boolean),
              ...(admin ? { offering_kind: values.offering_kind } : {}),
              supplier: values.supplier || null,
              ...(values.openapi_spec_url
                ? { openapi_spec_url: values.openapi_spec_url }
                : { openapi_spec_url: "" }),
            },
          });
          form.reset(values);
          toast.success(translateUi("Tool metadata saved"));
        } catch {
          toast.error(translateUi("Metadata update failed"));
        }
      })}
    >
      <fieldset disabled={mutation.isPending} className="space-y-3">
        {service.icon_url && (
          <Button
            type="button"
            variant="secondary"
            onClick={async () => {
              try {
                await mutation.mutateAsync({
                  serviceId: service.id,
                  body: { icon_url: "" },
                });
                toast.success(translateUi("Tool icon removed"));
              } catch {
                toast.error(translateUi("Icon removal failed"));
              }
            }}
          >{t("Remove icon")}</Button>
        )}
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="text-12">{t("Name")}<Input {...form.register("name")} />
          </label>
          <label className="text-12">{t("Supplier")}<Input {...form.register("supplier")} />
          </label>
          <label className="text-12">{t("Description")}<Input {...form.register("description")} />
          </label>
          {admin && (
            <label className="text-12">{t("Offering kind")}<Select
                value={form.watch("offering_kind")}
                onValueChange={(value) =>
                  form.setValue("offering_kind", value as "ai_service" | "tool")
                }
              >
                <SelectTrigger aria-label={t("Offering kind")}>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="ai_service">{t("AI Service")}</SelectItem>
                  <SelectItem value="tool">{t("Tool")}</SelectItem>
                </SelectContent>
              </Select>
            </label>
          )}
          <label className="text-12">{t("OpenAPI spec URL")}<Input {...form.register("openapi_spec_url")} />
          </label>
        </div>
        <details className="space-y-3">
          <summary className="cursor-pointer text-12">{t("Documentation and operational notes")}</summary>
          <div className="grid gap-3 sm:grid-cols-2">
            {(
              [
                "homepage_url",
                "repository_url",
                "issues_url",
                "examples_url",
                "auth_notes",
                "known_limitations",
              ] as const
            ).map((field) => (
              <label key={field} className="text-12">
                {field.replaceAll("_", " ")}
                <Input {...form.register(field)} />
              </label>
            ))}
          </div>
          <label className="block text-12">{t("Required permissions")}<Input
              placeholder={t("read:api, write:data")}
              {...form.register("required_permissions")}
            />
          </label>
          {form.formState.errors.required_permissions && (
            <p role="alert" className="text-11 text-destructive">
              {translateUi(form.formState.errors.required_permissions.message ?? "")}
            </p>
          )}
          <p className="text-11 text-text-tertiary">{t("Comma-separated vendor permissions required for the operations.")}</p>
          <p className="text-12 font-medium">{t("Capabilities")}</p>
          <div className="grid gap-2 sm:grid-cols-2">
            {(
              [
                ["supports_proxy_read", "Proxy read"],
                ["supports_proxy_write", "Proxy write"],
                ["supports_proxy_binary_upload", "Binary upload"],
                ["supports_direct_downstream_auth", "Direct downstream auth"],
                ["supports_authoring_via_nyx", "Authoring via NyxID"],
                ["supports_websocket", "WebSocket"],
                ["supports_streaming", "Streaming"],
              ] as const
            ).map(([key, label]) => (
              <label
                key={key}
                htmlFor={`tool-cap-${service.id}-${key}`}
                className="flex items-center justify-between rounded-lg border border-border p-2 text-12"
              >
                {label}
                <Switch
                  id={`tool-cap-${service.id}-${key}`}
                  checked={form.watch(`capabilities.${key}`)}
                  onCheckedChange={(value) =>
                    form.setValue(`capabilities.${key}`, value)
                  }
                />
              </label>
            ))}
          </div>
        </details>
        <p className="text-12">{t("Topics")}</p>
        <div className="flex flex-wrap gap-1">
          {topics.map((topic) => (
            <Button
              key={topic.slug}
              type="button"
              variant={
                (form.watch("topics") ?? []).includes(topic.slug)
                  ? "secondary"
                  : "ghost"
              }
              onClick={() =>
                form.setValue(
                  "topics",
                  (form.watch("topics") ?? []).includes(topic.slug)
                    ? (form.watch("topics") ?? []).filter(
                        (t) => t !== topic.slug,
                      )
                    : [...(form.watch("topics") ?? []), topic.slug],
                )
              }
            >
              {topic.label}
            </Button>
          ))}
        </div>
        {service.import_source && (
          <p className="text-11 text-text-tertiary">{t("Imported from ")}{service.import_source.kind}:{" "}
            {service.import_source.reference} ·{" "}
            {service.import_source.version ?? t("Unversioned")}
          </p>
        )}
        <div className="flex justify-end">
          <Button
            variant="primary"
            type="submit"
            disabled={!form.formState.isDirty}
            isLoading={mutation.isPending}
          >{t("Save metadata")}</Button>
        </div>
      </fieldset>
    </form>
  );
}
