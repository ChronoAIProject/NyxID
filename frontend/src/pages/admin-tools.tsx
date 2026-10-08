import { PublicationOperations } from "@/components/services/tool-publication";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { toast } from "sonner";
import { useServices } from "@/hooks/use-services";
import { useEndpoints } from "@/hooks/use-endpoints";
import { useToolTopics } from "@/hooks/use-tools";
import { useCatalogToolMutation } from "@/hooks/use-catalog-admin";
import { addToolSchema } from "@/schemas/tools";
import { useAuthStore } from "@/stores/auth-store";
import type { DownstreamService } from "@/types/api";
import { useAppForm } from "@/components/ui/form";
import { PageHeader } from "@/components/shared/page-header";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Skeleton } from "@/components/ui/skeleton";
import { CatalogToolMetadata } from "@/components/services/catalog-tool-metadata";

export function AdminToolsPage() {
  const { data: services = [], isLoading, error, refetch } = useServices();
  const admin = useAuthStore((state) => state.user?.is_admin) ?? false;
  const [adding, setAdding] = useState(false);
  const [createdTool, setCreatedTool] = useState<DownstreamService | null>(
    null,
  );
  const tools = services.filter((s) => s.offering_kind === "tool");
  const write = admin;
  const suppliers = Array.from(new Set(tools.map((s) => s.supplier ?? s.name)));
  if (!admin)
    return <ErrorBanner message="Platform admin authority required" />;
  return (
    <div className="space-y-6">
      <PageHeader
        title="Tools"
        description="Curate providers, operation publication, and import provenance."
        actions={
          write ? (
            <AddCtaButton label="Add tool" onClick={() => setAdding(true)} />
          ) : undefined
        }
      />
      {isLoading ? (
        <Skeleton className="h-48" />
      ) : error ? (
        <ErrorBanner message="Unable to load tool catalog" onRetry={refetch} />
      ) : (
        <Tabs defaultValue="providers" className="space-y-6">
          <TabsList>
            <TabsTrigger value="providers">Providers</TabsTrigger>
            <TabsTrigger value="tools">Tools</TabsTrigger>
          </TabsList>
          <TabsContent value="providers" className="space-y-4">
            {suppliers.map((supplier) => (
              <section
                key={supplier}
                className="rounded-xl border border-border/50 bg-card p-4"
              >
                <h2 className="text-15 font-semibold">{supplier}</h2>
                <p className="text-11 text-text-tertiary">
                  {
                    tools.filter((t) => (t.supplier ?? t.name) === supplier)
                      .length
                  }{" "}
                  tools
                </p>
                {tools
                  .filter((t) => (t.supplier ?? t.name) === supplier)
                  .map((tool) => (
                    <ProviderStatus key={tool.id} tool={tool} />
                  ))}
              </section>
            ))}
          </TabsContent>
          <TabsContent value="tools">
            <div className="overflow-x-auto rounded-xl border border-border/50">
              <table className="w-full text-left text-12">
                <thead className="bg-overlay text-text-tertiary">
                  <tr>
                    <th className="p-4">Tool</th>
                    <th className="p-4">Supplier</th>
                    <th className="p-4">Operations</th>
                  </tr>
                </thead>
                <tbody>
                  {tools.map((tool) => (
                    <tr
                      key={tool.id}
                      className="border-t border-border/50 align-top"
                    >
                      <td className="space-y-3 p-4">
                        <h2 className="text-15 font-semibold">{tool.name}</h2>
                        <Button variant="ghost" asChild>
                          <Link
                            to="/services/$serviceId/edit"
                            params={{ serviceId: tool.id }}
                          >
                            Service settings
                          </Link>
                        </Button>
                        <details>
                          <summary className="cursor-pointer">
                            Edit metadata
                          </summary>
                          <CatalogToolMetadata
                            service={tool}
                            disabled={!write}
                            admin={admin}
                          />
                        </details>
                      </td>
                      <td className="p-4">{tool.supplier ?? tool.name}</td>
                      <td className="min-w-80 p-4">
                        <PublicationOperations tool={tool} disabled={!write} />
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </TabsContent>
        </Tabs>
      )}
      {createdTool && <CreatedToolStatus tool={createdTool} />}
      {!isLoading && tools.length === 0 && (
        <p className="rounded-lg border border-dashed border-border p-6 text-12 text-muted-foreground">
          No tools yet. Add a tool and publish validated operations.
        </p>
      )}
      <AddToolDialog
        open={adding}
        onOpenChange={setAdding}
        admin={admin}
        services={services}
        onCreated={setCreatedTool}
      />
    </div>
  );
}
function ProviderStatus({ tool }: { tool: DownstreamService }) {
  const { data: operations = [] } = useEndpoints(tool.id);
  const published = operations.filter(
    (e) => e.publication === "published" && e.is_active,
  ).length;
  const drafts = operations.filter((e) => e.publication === "draft").length;
  const blocker =
    tool.auth_method !== "none" && !tool.credential_configured
      ? "Store credential"
      : !tool.openapi_spec_url
        ? "No spec"
        : published === 0
          ? "Publish operations"
          : "Ready";
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border/50 py-3 text-12">
      <span>{tool.name}</span>
      <span>
        {published} published · {drafts} draft
      </span>
      <span>
        Credential{" "}
        {tool.auth_method === "none"
          ? "not required"
          : tool.credential_configured
            ? "configured"
            : "missing"}
      </span>
      <Badge variant={blocker === "Ready" ? "success" : "warning"}>
        {blocker}
      </Badge>
    </div>
  );
}
function AddToolDialog({
  open,
  onOpenChange,
  admin,
  services,
  onCreated,
}: {
  services: readonly DownstreamService[];
  onCreated: (tool: DownstreamService) => void;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  admin: boolean;
}) {
  const { data: topics = [] } = useToolTopics();
  const mutation = useCatalogToolMutation();
  const form = useAppForm<z.infer<typeof addToolSchema>>({
    resolver: zodResolver(addToolSchema),
    defaultValues: {
      creation_mode: "twin",
      twin_of_service_id: "",
      name: "",
      slug: "",
      base_url: "",
      auth_method: "none",
      auth_key_name: "Authorization",
      offering_kind: "tool",
      topics: [],
      supplier: "",
      openapi_spec_url: "",
    },
  });
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add tool</DialogTitle>
          <DialogDescription>
            Create a tool, then configure its credential and pricing in Service
            settings.
          </DialogDescription>
        </DialogHeader>
        <form
          className="space-y-3"
          onSubmit={form.handleSubmit(async (values) => {
            const {
              creation_mode,
              twin_of_service_id,
              base_url,
              auth_method,
              auth_key_name,
              openapi_spec_url,
              ...metadata
            } = values;
            const body: Record<string, unknown> = {
              ...metadata,
              offering_kind: "tool",
            };
            if (creation_mode === "twin") {
              body.twin_of_service_id = twin_of_service_id;
            } else {
              Object.assign(body, {
                base_url,
                auth_method,
                auth_key_name,
                service_category: "internal",
                platform_key: {
                  enabled: true,
                  audience: "public",
                  allowed_owner_ids: [],
                },
              });
              if (openapi_spec_url) body.openapi_spec_url = openapi_spec_url;
            }
            try {
              const created = await mutation.mutateAsync({ body });
              onCreated(created);
              onOpenChange(false);
              form.reset();
              toast.success("Tool created");
            } catch {
              toast.error("Tool creation failed");
            }
          })}
        >
          <label className="block text-12">
            Creation method
            <select
              aria-label="Creation method"
              className="h-8 w-full rounded-lg border border-input bg-background"
              {...form.register("creation_mode")}
            >
              <option value="twin">From an existing catalog service</option>
              <option value="new">New service</option>
            </select>
          </label>
          {form.watch("creation_mode") === "twin" && (
            <label className="block text-12">
              Source service
              <select
                aria-label="Source service"
                className="h-8 w-full rounded-lg border border-input bg-background"
                {...form.register("twin_of_service_id")}
              >
                <option value="">Choose a service</option>
                {services
                  .filter(
                    (service) =>
                      service.service_type === "http" &&
                      service.offering_kind !== "tool" &&
                      [
                        "none",
                        "bearer",
                        "header",
                        "query",
                        "query_param",
                        "basic",
                      ].includes(service.auth_method),
                  )
                  .map((service) => (
                    <option key={service.id} value={service.id}>
                      {service.name} ({service.slug})
                    </option>
                  ))}
              </select>
              <p className="mt-1 text-11 text-muted-foreground">
                Copies transport and operations as drafts. Credentials stay with
                the source.
              </p>
            </label>
          )}
          {["name", "slug", "supplier"].map((name) => (
            <label key={name} className="block text-12">
              {name.replaceAll("_", " ")}
              <Input
                {...form.register(
                  name as "name" | "slug" | "supplier" | "openapi_spec_url",
                )}
              />
            </label>
          ))}
          {form.watch("creation_mode") === "new" && (
            <fieldset
              disabled={!admin}
              title={
                !admin
                  ? "Transport and credential configuration requires platform admin authority"
                  : undefined
              }
              className="space-y-3"
            >
              <label className="block text-12">
                Base URL
                <Input {...form.register("base_url")} />
              </label>
              <label className="block text-12">
                Auth method
                <select
                  {...form.register("auth_method")}
                  className="h-8 w-full rounded-lg border border-input bg-background"
                >
                  <option value="none">None</option>
                  <option value="bearer">Bearer</option>
                  <option value="header">Header</option>
                </select>
              </label>
              <label className="block text-12">
                Key header
                <Input {...form.register("auth_key_name")} />
              </label>
              <label className="block text-12">
                OpenAPI spec URL
                <Input {...form.register("openapi_spec_url")} />
              </label>
            </fieldset>
          )}
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
          <DialogFooter>
            <Button
              type="button"
              variant="ghost"
              onClick={() => onOpenChange(false)}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              type="submit"
              disabled={
                !form.formState.isDirty ||
                !form.watch("name") ||
                !form.watch("slug") ||
                (form.watch("creation_mode") === "new"
                  ? !form.watch("base_url")
                  : !form.watch("twin_of_service_id"))
              }
              isLoading={mutation.isPending}
            >
              Create tool
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function CreatedToolStatus({ tool }: { tool: DownstreamService }) {
  const { data: operations = [] } = useEndpoints(tool.id);
  return (
    <section
      className="rounded-xl border border-border/50 bg-card p-4 text-12"
      role="status"
    >
      <p>
        {tool.name} created ·{" "}
        {
          operations.filter((operation) => operation.publication === "draft")
            .length
        }{" "}
        draft operations
      </p>
      <div className="mt-3 flex gap-2">
        <Button variant="secondary" asChild>
          <Link to="/services/$serviceId/edit" params={{ serviceId: tool.id }}>
            Configure credential and pricing
          </Link>
        </Button>
        <Button variant="ghost" asChild>
          <Link to="/services/$serviceId" params={{ serviceId: tool.id }}>
            Review and publish operations
          </Link>
        </Button>
      </div>
    </section>
  );
}
