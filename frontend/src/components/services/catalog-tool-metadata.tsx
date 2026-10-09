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
import { toolMetadataFields } from "@/schemas/tools";
import { useToolTopics } from "@/hooks/use-tools";
import { useCatalogToolMutation } from "@/hooks/use-catalog-admin";
import type { DownstreamService } from "@/types/api";
const schema = z.object({
  ...toolMetadataFields,
  name: z.string().min(1).max(200),
  description: z.string().max(500),
  openapi_spec_url: z.url().or(z.literal("")),
});
export function CatalogToolMetadata({
  service,
}: {
  service: DownstreamService;
}) {
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
              ...values,
              supplier: values.supplier || null,
              ...(values.openapi_spec_url
                ? { openapi_spec_url: values.openapi_spec_url }
                : { openapi_spec_url: "" }),
            },
          });
          form.reset(values);
          toast.success("Tool metadata saved");
        } catch {
          toast.error("Metadata update failed");
        }
      })}
    >
      <fieldset disabled={mutation.isPending} className="space-y-3">
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
            <Select
              value={form.watch("offering_kind")}
              onValueChange={(value) =>
                form.setValue("offering_kind", value as "ai_service" | "tool")
              }
            >
              <SelectTrigger aria-label="Offering kind">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="ai_service">AI Service</SelectItem>
                <SelectItem value="tool">Tool</SelectItem>
              </SelectContent>
            </Select>
          </label>
          <label className="text-12">
            OpenAPI spec URL
            <Input {...form.register("openapi_spec_url")} />
          </label>
        </div>
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
