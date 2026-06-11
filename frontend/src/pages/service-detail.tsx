import { useEffect, useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { useForm } from "react-hook-form";
import {
  useService,
  useDeleteService,
  useTestSshConnection,
} from "@/hooks/use-services";
import {
  useAnonymousEndpoints,
  useCreateAnonymousEndpoint,
  useDeleteAnonymousEndpoint,
  useUpdateAnonymousEndpoint,
} from "@/hooks/use-anonymous-endpoints";
import { usePushNodeCredential } from "@/hooks/use-nodes";
import {
  isOidcService,
  isConnectable,
  getAuthTypeLabel,
  SERVICE_CATEGORY_LABELS,
  SERVICE_TYPE_LABELS,
} from "@/lib/constants";
import {
  SSH_AUTH_MODE_LABELS,
  getSshAuthModeBadgeVariant,
  inferSshAuthMode,
} from "@/lib/ssh-auth-mode";
import { formatDate } from "@/lib/utils";
import { buildStandaloneCredentialAcceptUrl } from "@/lib/credential-accept-url";
import { PageHeader } from "@/components/shared/page-header";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { DetailSection } from "@/components/shared/detail-section";
import { DetailRow } from "@/components/shared/detail-row";
import { CopyableField } from "@/components/shared/copyable-field";
import { DefaultHeadersEditor } from "@/components/shared/default-headers-editor";
import { OidcCredentialsSection } from "@/components/dashboard/oidc-credentials-section";
import { EndpointList } from "@/components/dashboard/endpoint-list";
import { McpConnectionInfo } from "@/components/dashboard/mcp-connection-info";
import { SshServiceInstructions } from "@/components/dashboard/ssh-service-instructions";
import { ServiceRequirementsView } from "@/components/dashboard/service-requirements-editor";
import { RoutingSection } from "@/components/dashboard/routing-section";
import { useMyProviderTokens } from "@/hooks/use-providers";
import { useDeveloperApps } from "@/hooks/use-developer-apps";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { ErrorBanner } from "@/components/shared/error-banner";
import { ApiError } from "@/lib/api-client";
import {
  Pencil,
  Trash2,
  Terminal,
  Router,
  ExternalLink,
  Send,
  Plus,
  Globe2,
} from "lucide-react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  pushNodeCredentialSchema,
  type PushNodeCredentialFormData,
  type PushNodeCredentialFormInput,
} from "@/schemas/nodes";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import {
  anonymousEndpointCreateSchema,
  type AnonymousEndpointRuleFormData,
  type AnonymousEndpointRuleFormInput,
} from "@/schemas/anonymous-endpoints";
import type { NodePendingCredentialInjectionMethod } from "@/types/nodes";
import type { AnonymousEndpointRule } from "@/types/api";
import { toast } from "sonner";

const PROPAGATION_MODE_LABELS: Readonly<Record<string, string>> = {
  none: "None",
  headers: "Headers (X-NyxID-*)",
  jwt: "Signed JWT",
  both: "Headers + JWT",
};

export function ServiceDetailPage() {
  const { serviceId } = useParams({ strict: false }) as { serviceId: string };
  const navigate = useNavigate();
  const { data: service, isLoading, error, refetch } = useService(serviceId);
  const deleteMutation = useDeleteService();
  const testSshMutation = useTestSshConnection();
  const { data: tokens } = useMyProviderTokens();
  const { data: appsData } = useDeveloperApps();

  useBreadcrumbLabel(service?.name);

  async function handleDelete() {
    if (!service) return;
    try {
      await deleteMutation.mutateAsync(service.id);
      toast.success("Service deleted successfully");
      void navigate({ to: "/services" });
    } catch {
      toast.error("Failed to delete service");
    }
  }

  async function handleTestSshConnection() {
    if (!service?.ssh_config) return;
    const principal = service.ssh_config.allowed_principals[0];
    if (!principal) {
      toast.error("No SSH principal is configured for this service");
      return;
    }
    try {
      await testSshMutation.mutateAsync({ serviceId: service.id, principal });
      toast.success("SSH connection test completed");
    } catch (err) {
      const message = err instanceof Error ? err.message : "SSH test failed";
      toast.error(message);
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-64 w-full" />
        <Skeleton className="h-48 w-full" />
      </div>
    );
  }

  if (error || !service) {
    return (
      <div className="space-y-8">
        <PageHeader title="Service Not Found" />
        <ErrorBanner
          message={
            error instanceof ApiError
              ? error.message
              : "The service you are looking for does not exist or has been deleted."
          }
          onRetry={refetch}
        />
      </div>
    );
  }

  const oidc = isOidcService(service);
  const isSshService = service.service_type === "ssh";
  const sshAuthMode = inferSshAuthMode(
    service.ssh_config?.ssh_auth_mode,
    service.ssh_config?.certificate_auth_enabled,
  );
  const terminalSupported =
    isSshService &&
    service.ssh_config != null &&
    sshAuthMode !== "proxy_only" &&
    service.ssh_config.allowed_principals.length > 0;

  return (
    <div className="space-y-8">
      <PageHeader
        title={service.name}
        description={service.description ?? undefined}
        actions={
          <>
            {terminalSupported && (
              <Button
                variant="outline"
                className="text-text-tertiary hover:text-muted-foreground"
                onClick={() =>
                  void navigate({
                    to: "/ssh/$serviceId/terminal",
                    params: { serviceId },
                    search: {
                      principal: service.ssh_config?.allowed_principals[0],
                    },
                  })
                }
              >
                <ButtonIcon>
                  <Terminal className="h-3 w-3" />
                </ButtonIcon>
                Terminal
              </Button>
            )}
            <Button
              variant="outline"
              className="text-text-tertiary hover:text-muted-foreground"
              onClick={() =>
                void navigate({
                  to: "/services/$serviceId/edit",
                  params: { serviceId },
                })
              }
            >
              <ButtonIcon>
                <Pencil className="h-3 w-3" />
              </ButtonIcon>
              Edit
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              <ButtonIcon variant="destructive">
                <Trash2 className="h-3 w-3 text-destructive" />
              </ButtonIcon>
              Delete
            </Button>
          </>
        }
      />

      <DetailSection title="General">
        <DetailRow label="Slug" value={service.slug} />
        <DetailRow
          label="Service Type"
          value={
            SERVICE_TYPE_LABELS[service.service_type] ?? service.service_type
          }
          badge
        />
        <DetailRow
          label="Category"
          value={
            SERVICE_CATEGORY_LABELS[service.service_category] ??
            service.service_category
          }
          badge
        />
        <DetailRow
          label="Visibility"
          value={
            service.visibility === "private"
              ? "Private (only visible to you)"
              : "Public"
          }
          badge
          badgeVariant={
            service.visibility === "private" ? "secondary" : "secondary"
          }
        />
        {service.developer_app_ids && service.developer_app_ids.length > 0 && (
          <div className="space-y-1 py-2">
            <p className="text-[12px] text-muted-foreground">
              Developer App Scoping
            </p>
            <div className="flex flex-wrap gap-1.5">
              {service.developer_app_ids.map((appId) => {
                const app = appsData?.clients?.find((c) => c.id === appId);
                return (
                  <Badge key={appId} variant="secondary">
                    {app?.client_name ?? appId}
                  </Badge>
                );
              })}
            </div>
            <p className="text-xs text-muted-foreground">
              Users who consent to these apps will have this service
              auto-provisioned.
            </p>
          </div>
        )}
        {!isSshService && (
          <>
            <DetailRow label="Base URL" value={service.base_url} copyable />
            <DetailRow
              label="Auth Type"
              value={getAuthTypeLabel(service)}
              badge
            />
          </>
        )}
        <DetailRow
          label="Status"
          value={service.is_active ? "Active" : "Inactive"}
          badge
          badgeVariant={service.is_active ? "success" : "secondary"}
        />
        <DetailRow label="Created" value={formatDate(service.created_at)} />
        <DetailRow label="Updated" value={formatDate(service.updated_at)} />
      </DetailSection>

      {/*
        Issue #416: Your Routing — viewer-scoped node binding for this
        catalog row. Mirrors the section on /keys/$id so admins can
        manage routing on whichever surface they happen to be on,
        rather than tunneling through "go look in AI Services".
      */}
      <Separator />
      <YourRoutingSection
        service={service}
        onBindClick={() =>
          void navigate({
            to: "/keys",
            search: { tab: "services", slug: service.slug },
          })
        }
      />

      {!isSshService && service.your_binding_count === 1 && service.node_id && (
        <>
          <Separator />
          <ServiceCredentialPushSection service={service} />
        </>
      )}

      {isSshService ? (
        <>
          <Separator />
          <DetailSection title="SSH Configuration">
            {service.ssh_config ? (
              <>
                <DetailRow
                  label="Target"
                  value={`${service.ssh_config.host}:${String(service.ssh_config.port)}`}
                  copyable
                />
                <DetailRow
                  label="SSH Auth Mode"
                  value={SSH_AUTH_MODE_LABELS[sshAuthMode] ?? "Proxy Only"}
                  badge
                  badgeVariant={getSshAuthModeBadgeVariant(sshAuthMode)}
                />
                <DetailRow
                  label="Certificate Auth"
                  value={
                    service.ssh_config.certificate_auth_enabled
                      ? "Enabled"
                      : "Transport only"
                  }
                  badge
                  badgeVariant={
                    service.ssh_config.certificate_auth_enabled
                      ? "success"
                      : "secondary"
                  }
                />
                {service.ssh_config.certificate_auth_enabled && (
                  <>
                    <DetailRow
                      label="Certificate TTL"
                      value={`${String(service.ssh_config.certificate_ttl_minutes)} minutes`}
                    />
                  </>
                )}
                <DetailRow
                  label="Allowed Principals"
                  value={service.ssh_config.allowed_principals.join(", ")}
                  copyable
                />
                {service.ssh_config.ca_public_key && (
                  <CopyableField
                    label="SSH CA Public Key"
                    value={service.ssh_config.ca_public_key}
                  />
                )}
                {(service.ssh_config.ssh_auth_mode ?? "proxy_only") ===
                  "node_key" && (
                  <div className="py-2">
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => void handleTestSshConnection()}
                      isLoading={testSshMutation.isPending}
                    >
                      Test Connection
                    </Button>
                  </div>
                )}
              </>
            ) : (
              <p className="text-[12px] text-muted-foreground">
                SSH configuration is missing for this service.
              </p>
            )}
          </DetailSection>

          {service.ssh_config && (
            <>
              <Separator />
              <DetailSection title="Connection Instructions">
                <SshServiceInstructions
                  serviceId={service.id}
                  serviceSlug={service.slug}
                  sshConfig={service.ssh_config}
                />
              </DetailSection>
            </>
          )}
        </>
      ) : (
        <>
          {oidc && (
            <>
              <Separator />
              <DetailSection title="OIDC Configuration">
                <OidcCredentialsSection
                  serviceId={service.id}
                  oauthClientId={service.oauth_client_id}
                />
              </DetailSection>
            </>
          )}

          {isConnectable(service) && !oidc && (
            <>
              <Separator />
              <DetailSection title="API Endpoints">
                <EndpointList
                  serviceId={service.id}
                  hasApiSpecUrl={
                    (service.openapi_spec_url ?? service.api_spec_url) !==
                      null &&
                    (service.openapi_spec_url ?? service.api_spec_url) !==
                      undefined
                  }
                />
              </DetailSection>

              <Separator />
              <DetailSection title="MCP Connection">
                <McpConnectionInfo />
              </DetailSection>
            </>
          )}

          {service.identity_propagation_mode &&
            service.identity_propagation_mode !== "none" && (
              <>
                <Separator />
                <DetailSection title="Identity Propagation">
                  <DetailRow
                    label="Mode"
                    value={
                      PROPAGATION_MODE_LABELS[
                        service.identity_propagation_mode
                      ] ?? service.identity_propagation_mode
                    }
                    badge
                  />
                  {service.identity_include_user_id && (
                    <DetailRow
                      label="User ID"
                      value="Included"
                      badge
                      badgeVariant="success"
                    />
                  )}
                  {service.identity_include_email && (
                    <DetailRow
                      label="Email"
                      value="Included"
                      badge
                      badgeVariant="success"
                    />
                  )}
                  {service.identity_include_name && (
                    <DetailRow
                      label="Display Name"
                      value="Included"
                      badge
                      badgeVariant="success"
                    />
                  )}
                  {service.identity_jwt_audience && (
                    <DetailRow
                      label="JWT Audience"
                      value={service.identity_jwt_audience}
                    />
                  )}
                </DetailSection>
              </>
            )}

          {service.forward_access_token && (
            <>
              <Separator />
              <DetailSection title="Forward Access Token">
                <DetailRow
                  label="Status"
                  value="Enabled"
                  badge
                  badgeVariant="success"
                />
              </DetailSection>
            </>
          )}

          {service.inject_delegation_token && (
            <>
              <Separator />
              <DetailSection title="Delegation Token Injection">
                <DetailRow
                  label="Status"
                  value="Enabled"
                  badge
                  badgeVariant="success"
                />
                <DetailRow
                  label="Token Scope"
                  value={service.delegation_token_scope || "llm:proxy"}
                />
              </DetailSection>
            </>
          )}

          {(service.homepage_url ||
            service.repository_url ||
            service.issues_url ||
            service.examples_url ||
            service.auth_notes ||
            service.known_limitations ||
            (service.required_permissions &&
              service.required_permissions.length > 0) ||
            (service.recommended_skills &&
              service.recommended_skills.length > 0) ||
            service.capabilities) && (
            <>
              <Separator />
              <DetailSection title="Service Metadata">
                {service.homepage_url && (
                  <DetailRow
                    label="Homepage"
                    value={service.homepage_url}
                    copyable
                  />
                )}
                {service.repository_url && (
                  <DetailRow
                    label="Repository"
                    value={service.repository_url}
                    copyable
                  />
                )}
                {service.issues_url && (
                  <DetailRow
                    label="Issues"
                    value={service.issues_url}
                    copyable
                  />
                )}
                {service.examples_url && (
                  <DetailRow
                    label="Skills & Examples"
                    value={service.examples_url}
                    copyable
                  />
                )}
                {service.auth_notes && (
                  <DetailRow label="Auth Notes" value={service.auth_notes} />
                )}
                {service.known_limitations && (
                  <DetailRow
                    label="Known Limitations"
                    value={service.known_limitations}
                  />
                )}
                {service.required_permissions &&
                  service.required_permissions.length > 0 && (
                    <DetailRow
                      label="Required Permissions"
                      value={service.required_permissions.join(", ")}
                    />
                  )}
                {service.recommended_skills &&
                  service.recommended_skills.length > 0 && (
                    <DetailRow
                      label="Recommended Skills"
                      value={service.recommended_skills.join(", ")}
                    />
                  )}
                {service.capabilities && (
                  <div className="space-y-1 py-2">
                    <p className="text-[12px] text-muted-foreground">
                      Capabilities
                    </p>
                    <div className="flex flex-wrap gap-1.5">
                      {Object.entries(service.capabilities)
                        .filter(([, v]) => v === true)
                        .map(([k]) => (
                          <Badge key={k} variant="secondary">
                            {k.replace(/^supports_/, "").replaceAll("_", " ")}
                          </Badge>
                        ))}
                      {Object.entries(service.capabilities).every(
                        ([, v]) => v !== true,
                      ) && (
                        <span className="text-xs text-muted-foreground">
                          None configured
                        </span>
                      )}
                    </div>
                  </div>
                )}
              </DetailSection>
            </>
          )}

          <Separator />
          <DetailSection title="Default request headers">
            {service.default_request_headers &&
            service.default_request_headers.length > 0 ? (
              <div className="space-y-2">
                <p className="text-xs text-muted-foreground">
                  Injected on every proxied request. Non-overridable entries
                  replace caller-supplied values; overridable ones yield to
                  them.
                </p>
                <DefaultHeadersEditor
                  value={service.default_request_headers.map((h) => ({ ...h }))}
                  onChange={() => {
                    /* read-only */
                  }}
                  readOnly
                />
              </div>
            ) : (
              <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
                No default headers configured for this service.
              </div>
            )}
          </DetailSection>

          <Separator />
          <AnonymousEndpointsSection serviceId={service.id} />

          <Separator />
          <DetailSection title="Provider Requirements">
            <ServiceRequirementsView
              serviceId={service.id}
              userTokenProviderIds={
                tokens ? new Set(tokens.map((t) => t.provider_id)) : undefined
              }
            />
          </DetailSection>
        </>
      )}
    </div>
  );
}

function injectionMethodForService(
  service: Pick<
    import("@/types/api").DownstreamService,
    "auth_method" | "auth_type"
  >,
): NodePendingCredentialInjectionMethod {
  if (service.auth_method === "query" || service.auth_type === "query") {
    return "query-param";
  }
  if (
    service.auth_method === "path" ||
    service.auth_method === "path-prefix" ||
    service.auth_type === "path"
  ) {
    return "path-prefix";
  }
  return "header";
}

function defaultFieldName(
  method: NodePendingCredentialInjectionMethod,
  authKeyName: string | null | undefined,
): string {
  if (authKeyName?.trim()) return authKeyName;
  if (method === "query-param") return "api_key";
  if (method === "path-prefix") return "api";
  return "Authorization";
}

function ServiceCredentialPushSection({
  service,
}: {
  readonly service: import("@/types/api").DownstreamService;
}) {
  const nodeId = service.node_id ?? "";
  const pushCredentialMutation = usePushNodeCredential(nodeId);
  const initialMethod = injectionMethodForService(service);
  const form = useForm<
    PushNodeCredentialFormInput,
    unknown,
    PushNodeCredentialFormData
  >({
    resolver: zodResolver(pushNodeCredentialSchema),
    defaultValues: {
      service_slug: service.slug,
      injection_method: initialMethod,
      field_name: defaultFieldName(initialMethod, service.auth_key_name),
      target_url: service.base_url ?? "",
      label: `${service.name} credential`,
      remote_crypto: true,
    },
  });
  const watchedFieldName = form.watch("field_name");

  async function handlePushCredential(data: PushNodeCredentialFormData) {
    if (!nodeId) return;
    try {
      const created = await pushCredentialMutation.mutateAsync(data);
      toast.success("Credential push created");
      window.location.assign(
        await buildStandaloneCredentialAcceptUrl(
          nodeId,
          created.id,
          `/services/${service.id}`,
        ),
      );
    } catch (err) {
      toast.error(
        err instanceof ApiError ? err.message : "Failed to push credential",
      );
    }
  }

  return (
    <DetailSection title="Push credential">
      <Form {...form}>
        <form
          className="space-y-4 p-5"
          onSubmit={(event) =>
            void form.handleSubmit(handlePushCredential)(event)
          }
        >
          <p className="text-[12px] text-muted-foreground">
            Create pending metadata for the node, then encrypt the secret in the
            browser on the accept page.
          </p>
          <div className="grid gap-4 md:grid-cols-2">
            <FormField
              control={form.control}
              name="injection_method"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Injection method</FormLabel>
                  <Select
                    value={field.value}
                    onValueChange={(value) => {
                      const method =
                        value as NodePendingCredentialInjectionMethod;
                      const previousDefault = defaultFieldName(
                        field.value,
                        service.auth_key_name,
                      );
                      const currentFieldName = form.getValues("field_name");
                      field.onChange(method);
                      if (
                        currentFieldName.trim() === "" ||
                        currentFieldName === previousDefault
                      ) {
                        form.setValue(
                          "field_name",
                          defaultFieldName(method, service.auth_key_name),
                          { shouldDirty: true, shouldValidate: true },
                        );
                      }
                    }}
                  >
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="header">Header</SelectItem>
                      <SelectItem value="query-param">Query param</SelectItem>
                      <SelectItem value="path-prefix">Path prefix</SelectItem>
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="field_name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Field name</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="target_url"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Target URL</FormLabel>
                  <FormControl>
                    <Input {...field} value={field.value ?? ""} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="label"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Label</FormLabel>
                  <FormControl>
                    <Input {...field} value={field.value ?? ""} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>
          <div className="flex justify-end">
            <Button
              variant="primary"
              type="submit"
              disabled={!watchedFieldName.trim() || !service.slug || !nodeId}
              isLoading={pushCredentialMutation.isPending}
            >
              <ButtonIcon variant="primary">
                <Send className="h-3 w-3" />
              </ButtonIcon>
              Push
            </Button>
          </div>
        </form>
      </Form>
    </DetailSection>
  );
}

function AnonymousEndpointsSection({
  serviceId,
}: {
  readonly serviceId: string;
}) {
  const { data: endpoints, isLoading } = useAnonymousEndpoints(serviceId);
  const createMutation = useCreateAnonymousEndpoint(serviceId);
  const updateMutation = useUpdateAnonymousEndpoint(serviceId);
  const deleteMutation = useDeleteAnonymousEndpoint(serviceId);
  const form = useForm<
    AnonymousEndpointRuleFormInput,
    unknown,
    AnonymousEndpointRuleFormData
  >({
    resolver: zodResolver(anonymousEndpointCreateSchema),
    defaultValues: {
      enabled: false,
      method: "GET",
      path_pattern: "/public/**",
      daily_quota: 1000,
    },
  });

  async function handleCreate(data: AnonymousEndpointRuleFormData) {
    try {
      await createMutation.mutateAsync(data);
      form.reset({
        enabled: false,
        method: "GET",
        path_pattern: "/public/**",
        daily_quota: 1000,
      });
      toast.success("Anonymous endpoint created");
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to create anonymous endpoint",
      );
    }
  }

  async function handleRuleUpdate(
    rule: AnonymousEndpointRule,
    data: Partial<AnonymousEndpointRuleFormData>,
  ) {
    try {
      await updateMutation.mutateAsync({ ruleId: rule.id, data });
      toast.success("Anonymous endpoint updated");
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to update anonymous endpoint",
      );
    }
  }

  async function handleDelete(ruleId: string) {
    try {
      await deleteMutation.mutateAsync(ruleId);
      toast.success("Anonymous endpoint deleted");
    } catch (err) {
      toast.error(
        err instanceof ApiError
          ? err.message
          : "Failed to delete anonymous endpoint",
      );
    }
  }

  return (
    <DetailSection title="Anonymous endpoints">
      <div className="space-y-5 p-5">
        <div className="flex items-center gap-2">
          <Globe2 className="h-4 w-4 text-primary" />
          <span className="text-[13px] font-medium">Public proxy rules</span>
        </div>

        <Form {...form}>
          <form
            className="grid gap-3 md:grid-cols-[120px_minmax(180px,1fr)_140px_auto]"
            onSubmit={(event) => void form.handleSubmit(handleCreate)(event)}
          >
            <FormField
              control={form.control}
              name="method"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Method</FormLabel>
                  <Select
                    value={field.value}
                    onValueChange={(value) => field.onChange(value)}
                  >
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      {PUBLIC_METHODS.map((method) => (
                        <SelectItem key={method} value={method}>
                          {method}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="path_pattern"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Path</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="daily_quota"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Daily quota</FormLabel>
                  <FormControl>
                    <Input
                      {...field}
                      type="number"
                      min={1}
                      value={
                        typeof field.value === "number" ||
                        typeof field.value === "string"
                          ? field.value
                          : ""
                      }
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <div className="flex items-end">
              <Button
                type="submit"
                variant="primary"
                isLoading={createMutation.isPending}
              >
                <ButtonIcon variant="primary">
                  <Plus className="h-3 w-3" />
                </ButtonIcon>
                Add
              </Button>
            </div>
          </form>
        </Form>

        <div className="space-y-2">
          {isLoading && <Skeleton className="h-12 w-full" />}
          {!isLoading && (!endpoints || endpoints.length === 0) && (
            <div className="rounded-lg bg-white/[0.03] px-4 py-3 text-[12px] text-muted-foreground">
              No anonymous endpoints configured.
            </div>
          )}
          {endpoints?.map((rule) => (
            <AnonymousEndpointRow
              key={rule.id}
              rule={rule}
              onUpdate={handleRuleUpdate}
              onDelete={(ruleId) => void handleDelete(ruleId)}
              isUpdating={updateMutation.isPending}
              isDeleting={deleteMutation.isPending}
            />
          ))}
        </div>
      </div>
    </DetailSection>
  );
}

const PUBLIC_METHODS = [
  "GET",
  "POST",
  "PUT",
  "PATCH",
  "DELETE",
  "HEAD",
  "OPTIONS",
] as const;

function AnonymousEndpointRow({
  rule,
  onUpdate,
  onDelete,
  isUpdating,
  isDeleting,
}: {
  readonly rule: AnonymousEndpointRule;
  readonly onUpdate: (
    rule: AnonymousEndpointRule,
    data: Partial<AnonymousEndpointRuleFormData>,
  ) => Promise<void>;
  readonly onDelete: (ruleId: string) => void;
  readonly isUpdating: boolean;
  readonly isDeleting: boolean;
}) {
  const [method, setMethod] = useState(rule.method);
  const [pathPattern, setPathPattern] = useState(rule.path_pattern);
  const [dailyQuota, setDailyQuota] = useState(String(rule.daily_quota));

  useEffect(() => {
    setMethod(rule.method);
    setPathPattern(rule.path_pattern);
    setDailyQuota(String(rule.daily_quota));
  }, [rule]);

  const dirty =
    method !== rule.method ||
    pathPattern !== rule.path_pattern ||
    Number(dailyQuota) !== rule.daily_quota;

  return (
    <div className="grid gap-3 rounded-lg border border-white/[0.08] bg-white/[0.02] p-3 md:grid-cols-[88px_120px_minmax(180px,1fr)_120px_auto_auto] md:items-center">
      <div className="flex items-center gap-2">
        <Switch
          checked={rule.enabled}
          onCheckedChange={(enabled) =>
            void onUpdate(rule, { enabled: Boolean(enabled) })
          }
          disabled={isUpdating}
        />
        <Badge variant={rule.enabled ? "success" : "secondary"}>
          {rule.enabled ? "Enabled" : "Draft"}
        </Badge>
      </div>
      <Select
        value={method}
        onValueChange={(value) =>
          setMethod(value as AnonymousEndpointRule["method"])
        }
      >
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {PUBLIC_METHODS.map((item) => (
            <SelectItem key={item} value={item}>
              {item}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Input
        value={pathPattern}
        onChange={(e) => setPathPattern(e.target.value)}
      />
      <Input
        type="number"
        min={1}
        value={dailyQuota}
        onChange={(e) => setDailyQuota(e.target.value)}
      />
      <Button
        size="sm"
        variant="outline"
        disabled={!dirty || isUpdating}
        isLoading={isUpdating}
        onClick={() =>
          void onUpdate(rule, {
            method,
            path_pattern: pathPattern,
            daily_quota: Number(dailyQuota),
          })
        }
      >
        Save
      </Button>
      <Button
        size="sm"
        variant="destructive"
        disabled={isDeleting}
        isLoading={isDeleting}
        onClick={() => onDelete(rule.id)}
      >
        <ButtonIcon variant="destructive">
          <Trash2 className="h-3 w-3 text-destructive" />
        </ButtonIcon>
        Delete
      </Button>
    </div>
  );
}

/**
 * Issue #416: viewer-scoped routing for an admin-catalog row.
 *
 * Three states driven by `service.your_binding_count`:
 *
 *   - `0`: viewer has no personal `UserService` for this catalog row.
 *     Show a "Bind in AI Services" CTA that hands off to `/keys` with
 *     the catalog slug pre-selected (the existing AddKeyDialog
 *     auto-open flow handles credential / endpoint / node selection).
 *   - `1`: exactly one personal binding -> render the editable
 *     `RoutingSection` so the admin can change routing in place. Same
 *     widget as `/keys/$id`; mutates the underlying `UserService`.
 *   - `>= 2`: multiple personal bindings -> show a disambiguation hint
 *     with a link to `/keys`. Picking arbitrarily would silently
 *     mutate one of the user's other bindings.
 */
function YourRoutingSection({
  service,
  onBindClick,
}: {
  readonly service: { readonly slug: string } & Pick<
    import("@/types/api").DownstreamService,
    "node_id" | "your_user_service_id" | "your_binding_count"
  >;
  readonly onBindClick: () => void;
}) {
  const count = service.your_binding_count ?? 0;
  const userServiceId = service.your_user_service_id ?? null;

  if (count === 1 && userServiceId) {
    return (
      <RoutingSection
        nodeId={service.node_id ?? null}
        serviceId={userServiceId}
        title="Your Routing"
        description="Your personal routing for this service. Other users have their own."
      />
    );
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Router className="h-4 w-4 text-primary" />
            <CardTitle className="text-[15px]">Your Routing</CardTitle>
          </div>
          <Button
            variant="outline"
            className="text-text-tertiary hover:text-muted-foreground"
            onClick={onBindClick}
          >
            {count === 0 ? "Bind in AI Services" : "Manage in AI Services"}
            <ButtonIcon>
              <ExternalLink className="h-3 w-3" />
            </ButtonIcon>
          </Button>
        </div>
        <CardDescription>
          Your personal routing for this service. Other users have their own.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {count === 0 ? (
          <p className="text-xs text-muted-foreground">
            You haven't bound this service to your account yet, so there's no
            routing to configure here.
          </p>
        ) : (
          <p className="text-xs text-muted-foreground">
            You have {String(count)} personal bindings for this service. Manage
            each one's routing in AI Services.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
