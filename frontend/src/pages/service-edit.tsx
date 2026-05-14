import { useEffect } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useService, useUpdateService } from "@/hooks/use-services";
import { useDeveloperApps } from "@/hooks/use-developer-apps";
import {
  updateServiceSchema,
  type UpdateServiceFormData,
  VISIBILITY_OPTIONS,
  type WsFrameInjection,
} from "@/schemas/services";
import type { DefaultRequestHeader } from "@/schemas/default-request-headers";
import { DefaultHeadersEditor } from "@/components/shared/default-headers-editor";
import { WsFrameInjectionsEditor } from "@/components/shared/ws-frame-injections-editor";
import {
  getAuthTypeLabel,
  SERVICE_CATEGORY_LABELS,
  SERVICE_TYPE_LABELS,
  VISIBILITY_LABELS,
} from "@/lib/constants";
import { parseAllowedPrincipals } from "@/lib/ssh";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { IdentityPropagationConfig } from "@/components/dashboard/identity-propagation-config";
import { Separator } from "@/components/ui/separator";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import { toast } from "sonner";

export function ServiceEditPage() {
  const { serviceId } = useParams({ strict: false }) as { serviceId: string };
  const navigate = useNavigate();
  const { data: service, isLoading, error, refetch } = useService(serviceId);
  const updateMutation = useUpdateService();
  const user = useAuthStore((s) => s.user);
  const { data: appsData } = useDeveloperApps();
  const developerApps = appsData?.clients?.filter((c) => c.is_active) ?? [];

  const form = useForm<UpdateServiceFormData>({
    resolver: zodResolver(updateServiceSchema),
    defaultValues: {
      service_type: "http",
      name: "",
      description: "",
      base_url: "",
      openapi_spec_url: "",
      asyncapi_spec_url: "",
      identity_propagation_mode: "none",
      identity_include_user_id: false,
      identity_include_email: false,
      identity_include_name: false,
      identity_jwt_audience: "",
      forward_access_token: false,
      inject_delegation_token: false,
      delegation_token_scope: "",
      homepage_url: "",
      repository_url: "",
      issues_url: "",
      auth_notes: "",
      known_limitations: "",
      required_permissions: "",
      examples_url: "",
      recommended_skills: "",
      developer_app_ids: [],
      supports_proxy_read: false,
      supports_proxy_write: false,
      supports_proxy_binary_upload: false,
      supports_direct_downstream_auth: false,
      supports_authoring_via_nyx: false,
      supports_websocket: false,
      supports_streaming: false,
      host: "",
      port: "22",
      certificate_auth_enabled: false,
      certificate_ttl_minutes: "30",
      allowed_principals: "",
      default_request_headers: [],
      ws_frame_injections: [],
    },
  });

  useEffect(() => {
    if (service) {
      form.reset({
        service_type: service.service_type === "ssh" ? "ssh" : "http",
        visibility: service.visibility === "private" ? "private" : "public",
        name: service.name,
        description: service.description ?? "",
        base_url: service.service_type === "http" ? service.base_url : "",
        openapi_spec_url:
          service.openapi_spec_url ?? service.api_spec_url ?? "",
        asyncapi_spec_url: service.asyncapi_spec_url ?? "",
        identity_propagation_mode:
          (service.identity_propagation_mode as UpdateServiceFormData["identity_propagation_mode"]) ??
          "none",
        identity_include_user_id: service.identity_include_user_id ?? false,
        identity_include_email: service.identity_include_email ?? false,
        identity_include_name: service.identity_include_name ?? false,
        identity_jwt_audience: service.identity_jwt_audience ?? "",
        forward_access_token: service.forward_access_token ?? false,
        inject_delegation_token: service.inject_delegation_token ?? false,
        delegation_token_scope: service.delegation_token_scope || "llm:proxy",
        homepage_url: service.homepage_url ?? "",
        repository_url: service.repository_url ?? "",
        issues_url: service.issues_url ?? "",
        auth_notes: service.auth_notes ?? "",
        known_limitations: service.known_limitations ?? "",
        required_permissions: service.required_permissions?.join(", ") ?? "",
        examples_url: service.examples_url ?? "",
        recommended_skills: service.recommended_skills?.join(", ") ?? "",
        developer_app_ids: [...(service.developer_app_ids ?? [])],
        supports_proxy_read: service.capabilities?.supports_proxy_read ?? false,
        supports_proxy_write: service.capabilities?.supports_proxy_write ?? false,
        supports_proxy_binary_upload: service.capabilities?.supports_proxy_binary_upload ?? false,
        supports_direct_downstream_auth: service.capabilities?.supports_direct_downstream_auth ?? false,
        supports_authoring_via_nyx: service.capabilities?.supports_authoring_via_nyx ?? false,
        supports_websocket: service.capabilities?.supports_websocket ?? false,
        supports_streaming: service.capabilities?.supports_streaming ?? false,
        host: service.ssh_config?.host ?? "",
        port: service.ssh_config ? String(service.ssh_config.port) : "22",
        certificate_auth_enabled:
          service.ssh_config?.certificate_auth_enabled ?? false,
        certificate_ttl_minutes: service.ssh_config
          ? String(service.ssh_config.certificate_ttl_minutes)
          : "30",
        allowed_principals:
          service.ssh_config?.allowed_principals.join(", ") ?? "",
        default_request_headers: service.default_request_headers
          ? service.default_request_headers.map((h) => ({ ...h }))
          : [],
        ws_frame_injections: service.ws_frame_injections
          ? service.ws_frame_injections.map((rule) => ({ ...rule }))
          : [],
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [service]);

  async function onSubmit(data: UpdateServiceFormData) {
    if (!service) return;
    // NyxID#356 tri-state encoding: only send `default_request_headers`
    // when the user actually changed the list. Omitting the field tells
    // the backend to leave it unchanged. `null` explicitly clears.
    const previousHeaders = service.default_request_headers ?? [];
    const nextHeaders = data.default_request_headers ?? [];
    const headersChanged =
      previousHeaders.length !== nextHeaders.length ||
      previousHeaders.some((prev, idx) => {
        const curr = nextHeaders[idx];
        if (!curr) return true;
        return (
          prev.name !== curr.name ||
          prev.value !== curr.value ||
          prev.overridable !== curr.overridable ||
          prev.sensitive !== curr.sensitive
        );
      });
    const defaultRequestHeadersPayload: null | DefaultRequestHeader[] | undefined =
      headersChanged
        ? nextHeaders.length === 0
          ? null
          : nextHeaders.map((h) => ({ ...h }))
        : undefined;

    try {
      await updateMutation.mutateAsync({
        serviceId: service.id,
        data:
          service.service_type === "ssh"
            ? {
                name: data.name,
                description: data.description || "",
                visibility: data.visibility,
                ssh_config: {
                  host: (data.host ?? "").trim(),
                  port: Number(data.port),
                  certificate_auth_enabled:
                    data.certificate_auth_enabled ?? false,
                  certificate_ttl_minutes: Number(
                    data.certificate_ttl_minutes || "30",
                  ),
                  allowed_principals: parseAllowedPrincipals(
                    data.allowed_principals,
                  ),
                },
              }
            : {
                name: data.name,
                description: data.description || "",
                visibility: data.visibility,
                base_url: data.base_url || "",
                openapi_spec_url: data.openapi_spec_url || "",
                asyncapi_spec_url: data.asyncapi_spec_url || "",
                identity_propagation_mode: data.identity_propagation_mode,
                identity_include_user_id: data.identity_include_user_id,
                identity_include_email: data.identity_include_email,
                identity_include_name: data.identity_include_name,
                identity_jwt_audience: data.identity_jwt_audience || "",
                forward_access_token: data.forward_access_token,
                inject_delegation_token: data.inject_delegation_token,
                delegation_token_scope: data.delegation_token_scope || "",
                homepage_url: data.homepage_url || "",
                repository_url: data.repository_url || "",
                issues_url: data.issues_url || "",
                auth_notes: data.auth_notes || "",
                known_limitations: data.known_limitations || "",
                required_permissions: (data.required_permissions || "")
                  .split(/[,\n]/)
                  .map((s) => s.trim())
                  .filter(Boolean),
                examples_url: data.examples_url || "",
                recommended_skills: (data.recommended_skills || "")
                  .split(/[,\n]/)
                  .map((s) => s.trim())
                  .filter(Boolean),
                developer_app_ids: data.developer_app_ids ?? [],
                capabilities: {
                  supports_proxy_read: data.supports_proxy_read ?? false,
                  supports_proxy_write: data.supports_proxy_write ?? false,
                  supports_proxy_binary_upload: data.supports_proxy_binary_upload ?? false,
                  supports_direct_downstream_auth: data.supports_direct_downstream_auth ?? false,
                  supports_authoring_via_nyx: data.supports_authoring_via_nyx ?? false,
                  supports_websocket: data.supports_websocket ?? false,
                  supports_streaming: data.supports_streaming ?? false,
                },
                ws_frame_injections: data.ws_frame_injections ?? [],
                ...(defaultRequestHeadersPayload !== undefined
                  ? { default_request_headers: defaultRequestHeadersPayload }
                  : {}),
              },
      });
      toast.success("Service updated");
      void navigate({
        to: "/services/$serviceId",
        params: { serviceId },
      });
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
        toast.error(err.message);
      } else {
        toast.error("Failed to update service");
      }
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-96 w-full" />
      </div>
    );
  }

  if (error || !service) {
    return (
      <div className="space-y-8">
        <PageHeader title="Service Not Found" />
        <ErrorBanner
          message={error instanceof ApiError ? error.message : "The service you are trying to edit does not exist or has been deleted."}
          onRetry={refetch}
        />
      </div>
    );
  }

  const isSshService = service.service_type === "ssh";
  const wsFrameRules = form.watch("ws_frame_injections") ?? [];
  const setWsFrameRules = (next: WsFrameInjection[]) =>
    form.setValue("ws_frame_injections", next, {
      shouldDirty: true,
      shouldValidate: true,
    });

  return (
    <div className="space-y-8">
      <PageHeader
        title={`Edit ${service.name}`}
      />

      <div className="max-w-2xl">
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {form.formState.errors.root && (
              <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
                {form.formState.errors.root.message}
              </div>
            )}

            <div className="flex flex-wrap gap-2">
              <Badge variant="secondary">
                {SERVICE_TYPE_LABELS[service.service_type] ??
                  service.service_type}
              </Badge>
              <Badge variant="secondary">
                {SERVICE_CATEGORY_LABELS[service.service_category] ??
                  service.service_category}
              </Badge>
            </div>

            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Service Name</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="description"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Description</FormLabel>
                  <FormControl>
                    <textarea
                      className="flex min-h-[80px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-[12px] placeholder:text-muted-foreground focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50"
                      placeholder="Optional description"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="visibility"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Visibility</FormLabel>
                  <Select
                    value={field.value ?? "public"}
                    onValueChange={field.onChange}
                  >
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue placeholder="Select visibility" />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      {VISIBILITY_OPTIONS.map((opt) => (
                        <SelectItem key={opt} value={opt}>
                          {VISIBILITY_LABELS[opt] ?? opt}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <p className="text-xs text-muted-foreground">
                    Private services are only visible to you.
                  </p>
                  <FormMessage />
                </FormItem>
              )}
            />

            {form.watch("visibility") === "private" &&
              user?.is_admin &&
              developerApps.length > 0 && (
                <div className="space-y-2">
                  <p className="text-[12px] font-medium">Developer App Scoping</p>
                  <p className="text-xs text-muted-foreground">
                    Select which developer apps grant access to this service.
                    Users who log in through a selected app will have this
                    service auto-provisioned in their AI Services.
                  </p>
                  <div className="space-y-2">
                    {developerApps.map((app) => {
                      const selected =
                        form.watch("developer_app_ids") ?? [];
                      const checked = selected.includes(app.id);
                      return (
                        <div
                          key={app.id}
                          className="flex items-center gap-2 rounded-lg border border-border p-2"
                        >
                          <Checkbox
                            id={`app-${app.id}`}
                            checked={checked}
                            onCheckedChange={(v) => {
                              const current =
                                form.getValues("developer_app_ids") ?? [];
                              form.setValue(
                                "developer_app_ids",
                                v
                                  ? [...current, app.id]
                                  : current.filter((id) => id !== app.id),
                              );
                            }}
                          />
                          <Label
                            htmlFor={`app-${app.id}`}
                            className="text-[12px] font-normal"
                          >
                            {app.client_name}
                          </Label>
                          <Badge variant="secondary" className="ml-auto text-xs">
                            {app.client_type}
                          </Badge>
                        </div>
                      );
                    })}
                  </div>
                </div>
              )}

            {isSshService ? (
              <>
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="host"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>SSH Host</FormLabel>
                        <FormControl>
                          <Input
                            placeholder="ssh.internal.example"
                            {...field}
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <FormField
                    control={form.control}
                    name="port"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>SSH Port</FormLabel>
                        <FormControl>
                          <Input type="number" min={1} max={65535} {...field} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>

                <div className="flex items-center justify-between rounded-lg border border-border p-3">
                  <Label
                    htmlFor="edit-ssh-cert-auth"
                    className="text-[12px] font-normal"
                  >
                    Enable short-lived SSH certificates
                  </Label>
                  <Switch
                    id="edit-ssh-cert-auth"
                    checked={form.watch("certificate_auth_enabled") ?? false}
                    onCheckedChange={(checked) =>
                      form.setValue("certificate_auth_enabled", checked)
                    }
                  />
                </div>

                {(form.watch("certificate_auth_enabled") ?? false) && (
                  <div className="grid gap-4 sm:grid-cols-2">
                    <FormField
                      control={form.control}
                      name="certificate_ttl_minutes"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>Certificate TTL (minutes)</FormLabel>
                          <FormControl>
                            <Input type="number" min={15} max={60} {...field} />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="allowed_principals"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>Allowed Principals</FormLabel>
                          <FormControl>
                            <Input placeholder="ubuntu, deploy" {...field} />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            Comma-separated SSH usernames NyxID is allowed to
                            sign.
                          </p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </div>
                )}
              </>
            ) : (
              <>
                <FormField
                  control={form.control}
                  name="base_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Base URL</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="https://api.example.com"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="openapi_spec_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>OpenAPI Spec URL</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="https://api.example.com/openapi.json"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Optional. Used to auto-discover API endpoints.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="asyncapi_spec_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>AsyncAPI Spec URL</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="https://api.example.com/asyncapi.json"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Optional. Used to document WebSocket and streaming
                        protocols.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <WsFrameInjectionsEditor
                  value={wsFrameRules}
                  onChange={setWsFrameRules}
                  errorMessage={
                    typeof form.formState.errors.ws_frame_injections
                      ?.message === "string"
                      ? form.formState.errors.ws_frame_injections.message
                      : undefined
                  }
                />

                <div>
                  <p className="mb-1 text-[12px] font-medium">Auth Type</p>
                  <Badge variant="secondary">{getAuthTypeLabel(service)}</Badge>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Auth type cannot be changed after creation.
                  </p>
                </div>

                {user && (
                  <>
                    <Separator className="my-2" />
                    <div className="space-y-2">
                      <h3 className="text-[13px] font-semibold">
                        Identity Propagation
                      </h3>
                      <p className="text-xs text-muted-foreground">
                        Configure how user identity is forwarded to this
                        downstream service during proxy requests.
                      </p>
                      <IdentityPropagationConfig
                        mode={form.watch("identity_propagation_mode") ?? "none"}
                        includeUserId={
                          form.watch("identity_include_user_id") ?? false
                        }
                        includeEmail={
                          form.watch("identity_include_email") ?? false
                        }
                        includeName={
                          form.watch("identity_include_name") ?? false
                        }
                        jwtAudience={form.watch("identity_jwt_audience") ?? ""}
                        onModeChange={(v) =>
                          form.setValue(
                            "identity_propagation_mode",
                            v as UpdateServiceFormData["identity_propagation_mode"],
                          )
                        }
                        onIncludeUserIdChange={(v) =>
                          form.setValue("identity_include_user_id", v)
                        }
                        onIncludeEmailChange={(v) =>
                          form.setValue("identity_include_email", v)
                        }
                        onIncludeNameChange={(v) =>
                          form.setValue("identity_include_name", v)
                        }
                        onJwtAudienceChange={(v) =>
                          form.setValue("identity_jwt_audience", v)
                        }
                      />
                    </div>

                    <Separator className="my-2" />
                    <div className="space-y-4">
                      <div className="space-y-1">
                        <h3 className="text-[13px] font-semibold">
                          Service Metadata
                        </h3>
                        <p className="text-xs text-muted-foreground">
                          Rich metadata for AI agent discovery. Helps agents
                          understand what this service is, where to find docs,
                          and what it supports.
                        </p>
                      </div>

                      <div className="grid gap-4 sm:grid-cols-2">
                        <FormField
                          control={form.control}
                          name="homepage_url"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>Homepage URL</FormLabel>
                              <FormControl>
                                <Input
                                  placeholder="https://docs.example.com"
                                  {...field}
                                />
                              </FormControl>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                        <FormField
                          control={form.control}
                          name="repository_url"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>Repository URL</FormLabel>
                              <FormControl>
                                <Input
                                  placeholder="https://github.com/org/repo"
                                  {...field}
                                />
                              </FormControl>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                        <FormField
                          control={form.control}
                          name="issues_url"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>Issues URL</FormLabel>
                              <FormControl>
                                <Input
                                  placeholder="https://github.com/org/repo/issues"
                                  {...field}
                                />
                              </FormControl>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                        <FormField
                          control={form.control}
                          name="examples_url"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>Skills & Examples URL</FormLabel>
                              <FormControl>
                                <Input
                                  placeholder="https://github.com/org/repo/tree/main/examples"
                                  {...field}
                                />
                              </FormControl>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                      </div>

                      <FormField
                        control={form.control}
                        name="auth_notes"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>Auth Notes</FormLabel>
                            <FormControl>
                              <Input
                                placeholder="Notes on downstream auth expectations..."
                                {...field}
                              />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />

                      <FormField
                        control={form.control}
                        name="known_limitations"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>Known Limitations</FormLabel>
                            <FormControl>
                              <Input
                                placeholder="Important caveats for agents and users..."
                                {...field}
                              />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />

                      <FormField
                        control={form.control}
                        name="required_permissions"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>Required Permissions</FormLabel>
                            <FormControl>
                              <Input
                                placeholder="read:api, write:data"
                                {...field}
                              />
                            </FormControl>
                            <p className="text-xs text-muted-foreground">
                              Comma-separated downstream permissions required
                              for key actions.
                            </p>
                            <FormMessage />
                          </FormItem>
                        )}
                      />

                      <FormField
                        control={form.control}
                        name="recommended_skills"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>Recommended Skills</FormLabel>
                            <FormControl>
                              <Input
                                placeholder="nyxid/ornn, ornn/authoring"
                                {...field}
                              />
                            </FormControl>
                            <p className="text-xs text-muted-foreground">
                              Comma-separated skill names/paths relevant for AI
                              tools.
                            </p>
                            <FormMessage />
                          </FormItem>
                        )}
                      />

                      <Separator className="my-2" />
                      <div className="space-y-2">
                        <div className="space-y-1">
                          <p className="text-[12px] font-medium">
                            Default request headers
                          </p>
                          <p className="text-xs text-muted-foreground">
                            Headers NyxID injects on every proxied request for
                            this service. Non-overridable headers replace
                            caller-supplied values; overridable ones yield to
                            them. Sensitive is a UI redaction flag only —
                            values are stored plaintext in v1, so do not place
                            real secrets here (use the service auth method
                            instead).
                          </p>
                        </div>
                        <FormField
                          control={form.control}
                          name="default_request_headers"
                          render={({ field, formState }) => {
                            // RHF stores nested array errors on the
                            // array field itself (an object with numeric
                            // keys). Without surfacing them, the editor
                            // would silently eat blank-name /
                            // invalid-value submits (see NyxID#356 code
                            // review P3): Save would block with no red
                            // state on the offending row. Flatten each
                            // row's first error into the editor's
                            // per-row `errors` map.
                            const rowErrs: Record<number, string> = {};
                            const nested = formState.errors
                              .default_request_headers as
                              | Record<
                                  string,
                                  {
                                    name?: { message?: string };
                                    value?: { message?: string };
                                    root?: { message?: string };
                                    message?: string;
                                  }
                                >
                              | undefined;
                            if (nested && typeof nested === "object") {
                              for (const [key, err] of Object.entries(nested)) {
                                const idx = Number(key);
                                if (!Number.isInteger(idx) || !err) continue;
                                const msg =
                                  err.name?.message ??
                                  err.value?.message ??
                                  err.root?.message ??
                                  err.message;
                                if (typeof msg === "string" && msg.length > 0) {
                                  rowErrs[idx] = msg;
                                }
                              }
                            }
                            return (
                              <FormItem>
                                <FormControl>
                                  <DefaultHeadersEditor
                                    value={field.value ?? []}
                                    onChange={(next) =>
                                      field.onChange(next.map((h) => ({ ...h })))
                                    }
                                    errors={rowErrs}
                                  />
                                </FormControl>
                                <FormMessage />
                              </FormItem>
                            );
                          }}
                        />
                      </div>

                      <div className="space-y-2">
                        <p className="text-[12px] font-medium">Capabilities</p>
                        <p className="text-xs text-muted-foreground">
                          Flags describing what this service supports through
                          NyxID proxy.
                        </p>
                        <div className="grid gap-2 sm:grid-cols-2">
                          {(
                            [
                              ["supports_proxy_read", "Proxy Read"],
                              ["supports_proxy_write", "Proxy Write"],
                              ["supports_proxy_binary_upload", "Binary Upload"],
                              ["supports_direct_downstream_auth", "Direct Downstream Auth"],
                              ["supports_authoring_via_nyx", "Authoring via NyxID"],
                              ["supports_websocket", "WebSocket"],
                              ["supports_streaming", "Streaming"],
                            ] as const
                          ).map(([key, label]) => (
                            <div
                              key={key}
                              className="flex items-center justify-between rounded-lg border border-border p-2"
                            >
                              <Label
                                htmlFor={`cap-${key}`}
                                className="text-xs font-normal"
                              >
                                {label}
                              </Label>
                              <Switch
                                id={`cap-${key}`}
                                checked={form.watch(key) ?? false}
                                onCheckedChange={(v) =>
                                  form.setValue(key, v)
                                }
                              />
                            </div>
                          ))}
                        </div>
                      </div>
                    </div>

                    <Separator className="my-2" />
                    <div className="space-y-4">
                      <div className="space-y-1">
                        <h3 className="text-[13px] font-semibold">
                          Forward Access Token
                        </h3>
                        <p className="text-xs text-muted-foreground">
                          Forward the caller&apos;s NyxID access token as
                          Authorization: Bearer to this service.
                        </p>
                      </div>

                      <div className="flex items-center justify-between rounded-lg border border-border p-3">
                        <Label
                          htmlFor="forward-access-token"
                          className="text-[12px] font-normal"
                        >
                          Forward Access Token
                        </Label>
                        <Switch
                          id="forward-access-token"
                          checked={
                            form.watch("forward_access_token") ?? false
                          }
                          onCheckedChange={(v) =>
                            form.setValue("forward_access_token", v)
                          }
                        />
                      </div>
                    </div>

                    <Separator className="my-2" />
                    <div className="space-y-4">
                      <div className="space-y-1">
                        <h3 className="text-[13px] font-semibold">
                          Delegation Token Injection
                        </h3>
                        <p className="text-xs text-muted-foreground">
                          When enabled, NyxID injects a short-lived delegation
                          token (X-NyxID-Delegation-Token) when proxying
                          requests to this service. The downstream service can
                          use this token to call NyxID APIs (e.g., LLM gateway)
                          on behalf of the user.
                        </p>
                      </div>

                      <div className="flex items-center justify-between rounded-lg border border-border p-3">
                        <Label
                          htmlFor="inject-delegation-token"
                          className="text-[12px] font-normal"
                        >
                          Inject delegation token
                        </Label>
                        <Switch
                          id="inject-delegation-token"
                          checked={
                            form.watch("inject_delegation_token") ?? false
                          }
                          onCheckedChange={(v) =>
                            form.setValue("inject_delegation_token", v)
                          }
                        />
                      </div>

                      {form.watch("inject_delegation_token") && (
                        <FormField
                          control={form.control}
                          name="delegation_token_scope"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>Delegation Token Scope</FormLabel>
                              <FormControl>
                                <Input placeholder="llm:proxy" {...field} />
                              </FormControl>
                              <p className="text-xs text-muted-foreground">
                                Space-separated scopes for the delegation token.
                                Defaults to &quot;llm:proxy&quot; if left empty.
                                Available scopes: llm:proxy, proxy:*, llm:status
                              </p>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                      )}
                    </div>
                  </>
                )}
              </>
            )}

            <div className="flex items-center justify-end gap-3 pt-4">
              <Button variant="primary" type="submit" isLoading={updateMutation.isPending} disabled={!form.formState.isDirty}>
                Save changes
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() =>
                  void navigate({
                    to: "/services/$serviceId",
                    params: { serviceId },
                  })
                }
              >
                Cancel
              </Button>
            </div>
          </form>
        </Form>
      </div>
    </div>
  );
}
