import { z } from "zod";
import { zodResolver } from "@hookform/resolvers/zod";
import { toast } from "sonner";
import { useAppForm } from "@/components/ui/form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { toolMetadataFields } from "@/schemas/tools";
import { useToolTopics } from "@/hooks/use-tools";
import { useCatalogToolMutation } from "@/hooks/use-catalog-admin";
import type { DownstreamService } from "@/types/api";
const schema = z.object({
  ...toolMetadataFields,
  name: z.string().min(1).max(200),
  description: z.string().max(500),
  openapi_spec_url: z.url().or(z.literal("")),
  visibility: z.enum(["public", "private"]),
  asyncapi_spec_url: z.url().or(z.literal("")),
  homepage_url: z.url().or(z.literal("")),
  repository_url: z.url().or(z.literal("")),
  issues_url: z.url().or(z.literal("")),
  examples_url: z.url().or(z.literal("")),
  auth_notes: z.string(),
  known_limitations: z.string(),
  required_permissions: z.string(),
  capabilities: z.string().refine((value) => {
    try {
      return !value || typeof JSON.parse(value) === "object";
    } catch {
      return false;
    }
  }, "Must be a JSON object"),
});
export function CatalogToolMetadata({
  service,
  disabled = false,
}: {
  service: DownstreamService;
  disabled?: boolean;
}) {
  const { data: topics = [] } = useToolTopics();
  const mutation = useCatalogToolMutation();
  const form = useAppForm<z.infer<typeof schema>>({
    resolver: zodResolver(schema),
    defaultValues: {
      visibility: service.visibility === "private" ? "private" : "public",
      asyncapi_spec_url: service.asyncapi_spec_url ?? "",
      homepage_url: service.homepage_url ?? "",
      repository_url: service.repository_url ?? "",
      issues_url: service.issues_url ?? "",
      examples_url: service.examples_url ?? "",
      auth_notes: service.auth_notes ?? "",
      known_limitations: service.known_limitations ?? "",
      required_permissions: (service.required_permissions ?? []).join("\n"),
      capabilities: service.capabilities
        ? JSON.stringify(service.capabilities)
        : "",
      name: service.name,
      description: service.description ?? "",
      offering_kind: service.offering_kind ?? "ai_service",
      topics: [...(service.topics ?? [])],
      supplier: service.supplier ?? "",
      openapi_spec_url: service.openapi_spec_url ?? "",
    },
  });
  return (
    <form
      className="space-y-3"
      onSubmit={form.handleSubmit(async (values) => {
        try {
          await mutation.mutateAsync({
            serviceId: service.id,
            body: Object.fromEntries(
              Object.entries({
                ...values,
                required_permissions: values.required_permissions
                  .split("\n")
                  .map((s) => s.trim())
                  .filter(Boolean),
                capabilities: values.capabilities
                  ? JSON.parse(values.capabilities)
                  : null,
              }).filter(
                ([field, value]) => !field.endsWith("_url") || value !== "",
              ),
            ),
          });
          form.reset(values);
          toast.success("Tool metadata saved");
        } catch {
          toast.error("Metadata update failed");
        }
      })}
    >
      <fieldset disabled={disabled || mutation.isPending} className="space-y-3">
        <div className="grid gap-3 sm:grid-cols-2">
          <label className="text-12">
            Name
            <Input {...form.register("name")} />
          </label>
          <label className="text-12">
            Supplier
            <Input {...form.register("supplier")} />
          </label>
          <label className="text-12">
            Description
            <Input {...form.register("description")} />
          </label>
          <label className="text-12">
            Offering kind
            <select
              {...form.register("offering_kind")}
              className="h-8 w-full rounded-lg border border-input bg-background text-12"
            >
              <option value="ai_service">AI Service</option>
              <option value="tool">Tool</option>
            </select>
          </label>
          <label className="text-12">
            OpenAPI spec URL
            <Input {...form.register("openapi_spec_url")} />
          </label>
        </div>
        <details>
          <summary className="cursor-pointer text-12">
            Documentation and visibility
          </summary>
          <div className="mt-3 grid gap-3 sm:grid-cols-2">
            <label className="text-12">
              Visibility
              <select
                {...form.register("visibility")}
                className="h-8 w-full rounded-lg border border-input bg-background"
              >
                <option value="public">Public</option>
                <option value="private">Private</option>
              </select>
            </label>
            {(
              [
                "asyncapi_spec_url",
                "homepage_url",
                "repository_url",
                "issues_url",
                "examples_url",
                "auth_notes",
                "known_limitations",
                "required_permissions",
                "capabilities",
              ] as const
            ).map((field) => (
              <label key={field} className="text-12">
                {field.replaceAll("_", " ")}
                <Input {...form.register(field)} />
              </label>
            ))}
          </div>
        </details>
        <p className="text-12">Topics</p>
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
          <p className="text-11 text-text-tertiary">
            Imported from {service.import_source.kind}:{" "}
            {service.import_source.reference} ·{" "}
            {service.import_source.version ?? "Unversioned"}
          </p>
        )}
        <div className="flex justify-end">
          <Button
            variant="primary"
            type="submit"
            disabled={!form.formState.isDirty}
            isLoading={mutation.isPending}
          >
            Save metadata
          </Button>
        </div>
      </fieldset>
    </form>
  );
}
