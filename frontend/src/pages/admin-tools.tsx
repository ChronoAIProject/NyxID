import { PublicationOperations } from "@/components/services/tool-publication";
import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { z } from "zod";
import { toast } from "sonner";
import { useServices } from "@/hooks/use-services";
import { useEndpoints } from "@/hooks/use-endpoints";
import { useToolEditorAuthority, useToolTopics } from "@/hooks/use-tools";
import {
  useCatalogToolMutation,
  useSpecOverlay,
  useImportOverlay,
} from "@/hooks/use-catalog-admin";
import {
  addToolSchema,
  importOverlaySchema,
  overlayDocumentSchema,
} from "@/schemas/tools";
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
  const { data: authority } = useToolEditorAuthority();
  const [adding, setAdding] = useState(false);
  const tools = services.filter((s) => s.offering_kind === "tool");
  const write = authority?.write ?? false;
  const suppliers = Array.from(new Set(tools.map((s) => s.supplier ?? s.name)));
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
            <TabsTrigger value="imports">Imports</TabsTrigger>
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
          <TabsContent value="imports" className="space-y-4">
            {tools.map((tool) => (
              <OverlayImport key={tool.id} tool={tool} disabled={!write} />
            ))}
          </TabsContent>
        </Tabs>
      )}
      {!isLoading && tools.length === 0 && (
        <p className="rounded-lg border border-dashed border-border p-6 text-12 text-muted-foreground">
          No tools yet. Add a tool, import an operation contract, and publish
          validated operations.
        </p>
      )}
      <AddToolDialog
        open={adding}
        onOpenChange={setAdding}
        admin={authority?.admin ?? false}
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
function OverlayImport({
  tool,
  disabled,
}: {
  tool: DownstreamService;
  disabled: boolean;
}) {
  const { data: overlay } = useSpecOverlay(tool.id);
  const mutation = useImportOverlay();
  const [document, setDocument] = useState<Record<string, unknown> | null>(
    null,
  );
  const [fileError, setFileError] = useState("");
  const form = useAppForm<z.infer<typeof importOverlaySchema>>({
    resolver: zodResolver(importOverlaySchema),
    defaultValues: { kind: "manual", reference: "", version: "" },
  });
  return (
    <section className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <h2 className="text-15 font-semibold">{tool.name}</h2>
      <form
        className="space-y-3"
        onSubmit={form.handleSubmit(async (values) => {
          if (!document) return;
          try {
            await mutation.mutateAsync({
              serviceId: tool.id,
              document,
              source: {
                kind: values.kind,
                reference: values.reference,
                version: values.version || null,
              },
            });
            setDocument(null);
            toast.success("Overlay imported");
          } catch {
            toast.error("Import failed");
          }
        })}
      >
        <fieldset
          disabled={disabled || mutation.isPending}
          className="space-y-3"
        >
          <label className="block text-12">
            OpenAPI JSON
            <Input
              type="file"
              accept="application/json,.json"
              onChange={async (e) => {
                try {
                  const file = e.target.files?.[0];
                  if (!file) return;
                  if (file.size > 1024 * 1024)
                    throw new Error("Overlay exceeds 1 MiB");
                  setDocument(
                    overlayDocumentSchema.parse(JSON.parse(await file.text())),
                  );
                  setFileError("");
                } catch (error) {
                  setDocument(null);
                  setFileError(
                    error instanceof Error ? error.message : "Invalid JSON",
                  );
                }
              }}
            />
          </label>
          <div className="grid gap-3 sm:grid-cols-3">
            <label className="text-12">
              Source
              <select
                className="h-8 w-full rounded-lg border border-input bg-background text-12"
                {...form.register("kind")}
              >
                <option value="manual">Manual</option>
                <option value="monid">Monid</option>
                <option value="vendor_spec">Vendor spec</option>
              </select>
            </label>
            <label className="text-12">
              Reference
              <Input {...form.register("reference")} />
            </label>
            <label className="text-12">
              Version
              <Input {...form.register("version")} />
            </label>
          </div>
          {fileError && (
            <p role="alert" className="text-12 text-destructive">
              {fileError}
            </p>
          )}
          <div className="flex justify-end">
            <Button
              type="submit"
              variant="primary"
              disabled={!document}
              isLoading={mutation.isPending}
            >
              Import and sync
            </Button>
          </div>
        </fieldset>
      </form>
      {mutation.data && (
        <p className="text-11">
          {mutation.data.operations_added} operations added ·{" "}
          {mutation.data.operations_changed} changed
        </p>
      )}
      {overlay && (
        <div className="space-y-2 text-11">
          <p>
            Revision {overlay.revision} · {overlay.operations_synced ?? ""}{" "}
            operations synced
          </p>
          <code className="break-all">SHA-256 {overlay.sha256}</code>
          <details>
            <summary>Current revision</summary>
            <pre className="max-h-80 overflow-auto rounded-lg bg-overlay p-3">
              {JSON.stringify(overlay.document, null, 2)}
            </pre>
          </details>
          {overlay.previous_document && (
            <details>
              <summary>Previous revision</summary>
              <pre className="max-h-80 overflow-auto rounded-lg bg-overlay p-3">
                {JSON.stringify(overlay.previous_document, null, 2)}
              </pre>
            </details>
          )}
        </div>
      )}
    </section>
  );
}
function AddToolDialog({
  open,
  onOpenChange,
  admin,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  admin: boolean;
}) {
  const { data: topics = [] } = useToolTopics();
  const mutation = useCatalogToolMutation();
  const importer = useImportOverlay();
  const [upload, setUpload] = useState<Record<string, unknown> | null>(null);
  const [uploadError, setUploadError] = useState("");
  const form = useAppForm<z.infer<typeof addToolSchema>>({
    resolver: zodResolver(addToolSchema),
    defaultValues: {
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
            settings. Import an overlay in Imports.
          </DialogDescription>
        </DialogHeader>
        <form
          className="space-y-3"
          onSubmit={form.handleSubmit(async (values) => {
            const { base_url, auth_method, auth_key_name, ...metadata } =
              values;
            const body: Record<string, unknown> = {
              ...metadata,
              offering_kind: "tool",
            };
            if (!body.openapi_spec_url) delete body.openapi_spec_url;
            if (admin)
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
            try {
              const created = await mutation.mutateAsync({ body });
              if (upload)
                await importer.mutateAsync({
                  serviceId: created.id,
                  document: upload,
                  source: {
                    kind: "manual",
                    reference: "Admin Tools upload",
                    version: null,
                  },
                });
              setUpload(null);
              onOpenChange(false);
              form.reset();
              toast.success("Tool created");
            } catch {
              toast.error("Tool creation failed");
            }
          })}
        >
          {["name", "slug", "supplier", "openapi_spec_url"].map((name) => (
            <label key={name} className="block text-12">
              {name.replaceAll("_", " ")}
              <Input
                {...form.register(
                  name as "name" | "slug" | "supplier" | "openapi_spec_url",
                )}
              />
            </label>
          ))}
          <label className="block text-12">
            Or upload OpenAPI JSON
            <Input
              type="file"
              accept="application/json,.json"
              onChange={async (event) => {
                try {
                  const file = event.target.files?.[0];
                  if (!file) {
                    setUpload(null);
                    return;
                  }
                  if (file.size > 1024 * 1024)
                    throw new Error("Overlay exceeds 1 MiB");
                  setUpload(
                    overlayDocumentSchema.parse(JSON.parse(await file.text())),
                  );
                  setUploadError("");
                } catch (error) {
                  setUpload(null);
                  setUploadError(
                    error instanceof Error ? error.message : "Invalid JSON",
                  );
                }
              }}
            />
          </label>
          {uploadError && (
            <p role="alert" className="text-12 text-destructive">
              {uploadError}
            </p>
          )}
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
          </fieldset>
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
                (admin && !form.watch("base_url"))
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
