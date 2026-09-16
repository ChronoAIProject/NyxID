import { PlatformServiceFields } from "@/components/services/platform-service-fields";
import {
  serviceFormPatch,
  serviceFormPayload,
  serviceFormValues,
} from "./service-edit.helpers";
import { useState } from "react";
import type { DownstreamService, UpdateServicePayload } from "@/types/api";
import { describeChanges, sameValue } from "@/lib/form-changes";
import {
  useChangeReview,
  useEditorMounted,
} from "@/components/shared/change-review-dialog";
import { StaleFormNotice } from "@/components/shared/stale-form-notice";
import { useNavigate, useParams } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { useService, useUpdateService } from "@/hooks/use-services";
import { useDeveloperApps } from "@/hooks/use-developer-apps";
import {
  DELEGATION_TOKEN_SCOPES,
  SSH_AUTH_MODES,
  updateServiceSchema,
  type UpdateServiceFormData,
  VISIBILITY_OPTIONS,
  type WsFrameInjection,
} from "@/schemas/services";
import { DefaultHeadersEditor } from "@/components/shared/default-headers-editor";
import { WsFrameInjectionsEditor } from "@/components/shared/ws-frame-injections-editor";
import {
  getAuthTypeLabel,
  SERVICE_CATEGORY_LABELS,
  SERVICE_TYPE_LABELS,
  VISIBILITY_LABELS,
} from "@/lib/constants";
import {
  SSH_AUTH_MODE_LABELS,
  getSshAuthModeChangeWarning,
  inferSshAuthMode,
} from "@/lib/ssh-auth-mode";
import { flattenRowErrors, flattenRowFieldErrors } from "@/lib/form-errors";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { IdentityPropagationConfig } from "@/components/dashboard/identity-propagation-config";
import { Separator } from "@/components/ui/separator";
import {
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  FormSubmitErrors,
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
  const { data: service, isLoading, error, refetch } = useService(serviceId);
  if (isLoading && !service) return <Skeleton className="h-96 w-full" />;
  if (!service)
    return (
      <ErrorBanner
        message={
          error instanceof ApiError ? error.message : "Unable to load service"
        }
        onRetry={refetch}
      />
    );
  return <ServiceEditForm key={serviceId} source={service} />;
}

function ServiceEditForm({ source }: { readonly source: DownstreamService }) {
  const [service, setService] = useState(source);
  const serviceId = service.id;
  const isMounted = useEditorMounted();
  const navigate = useNavigate();
  const updateMutation = useUpdateService();
  const user = useAuthStore((s) => s.user);
  const { data: appsData } = useDeveloperApps();
  const selectedAppIds = service.developer_app_ids ?? [];
  const developerApps = (appsData?.clients ?? []).filter(
    (c) => c.is_active || selectedAppIds.includes(c.id),
  );
  const unavailableAppIds = selectedAppIds.filter(
    (id) => !developerApps.some((app) => app.id === id),
  );

  const form = useAppForm<UpdateServiceFormData>({
    resolver: zodResolver(updateServiceSchema),
    defaultValues: serviceFormValues(service),
  });
  const stale =
    !sameValue(serviceFormValues(service), serviceFormValues(source)) ||
    (!!form.watch("credential")?.trim() &&
      service.updated_at !== source.updated_at);
  type ReviewedService = { serviceId: string; data: UpdateServicePayload };
  const review = useChangeReview<ReviewedService>(
    saveChanges,
    stale,
    serviceId,
  );

  function onSubmit(data: UpdateServiceFormData) {
    if (stale) return;
    const before = serviceFormPayload(serviceFormValues(service), service);
    const patch = serviceFormPatch(data, service);
    if (!user?.is_admin) {
      delete patch.inference;
      delete patch.platform_key;
      delete patch.credential;
    }
    const warning = patch.ssh_config
      ? getSshAuthModeChangeWarning(
          inferSshAuthMode(
            service.ssh_config?.ssh_auth_mode,
            service.ssh_config?.certificate_auth_enabled,
          ),
          inferSshAuthMode(
            patch.ssh_config.ssh_auth_mode,
            patch.ssh_config.certificate_auth_enabled,
          ),
        )
      : null;
    review.review({ serviceId, data: patch }, [
      ...(warning
        ? [{ field: "SSH mode transition", before: "Node Key", after: warning }]
        : []),
      ...describeChanges(before, patch, {
        secretFields: [
          "credential",
          "default_request_headers",
          "ws_frame_injections",
        ],
      }),
    ]);
  }

  async function saveChanges({ serviceId: targetId, data }: ReviewedService) {
    try {
      await updateMutation.mutateAsync({ serviceId: targetId, data });
      if (!isMounted()) return;
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
      throw err;
    }
  }

  const isSshService = service.service_type === "ssh";
  const wsFrameRules = form.watch("ws_frame_injections") ?? [];
  const setWsFrameRules = (next: WsFrameInjection[]) =>
    form.setValue("ws_frame_injections", next, {
      // This fires per keystroke from the editor's text inputs, and the
      // default shouldValidate would re-parse the whole service schema on
      // each one. Validate eagerly only while correcting an existing
      // error (so highlights clear live); otherwise defer to submit.
      shouldValidate: Boolean(form.formState.errors.ws_frame_injections),
    });

  return (
    <div className="space-y-8">
      <PageHeader title={`Edit ${service.name}`} />

      {stale && (
        <StaleFormNotice
          onReload={() => {
            setService(source);
            form.reset(serviceFormValues(source));
            review.cancel();
          }}
        />
      )}
      {review.dialog}
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

            {(form.watch("visibility") === "private" ||
              selectedAppIds.length > 0) &&
              user?.is_admin &&
              (developerApps.length > 0 || unavailableAppIds.length > 0) && (
                <div className="space-y-2">
                  <p className="text-[12px] font-medium">
                    Developer App Scoping
                  </p>
                  <p className="text-xs text-muted-foreground">
                    Select which developer apps grant access to this service.
                    Users who log in through a selected app will have this
                    service auto-provisioned in their AI Services. Remove
                    inactive applications before saving changes to this
                    selection.
                  </p>
                  <div className="space-y-2">
                    {unavailableAppIds.map((id) => (
                      <div key={id} className="flex items-center gap-2">
                        <Checkbox
                          id={`app-${id}`}
                          checked={(
                            form.watch("developer_app_ids") ?? []
                          ).includes(id)}
                          onCheckedChange={(checked) => {
                            const ids =
                              form.getValues("developer_app_ids") ?? [];
                            form.setValue(
                              "developer_app_ids",
                              checked
                                ? [...ids, id]
                                : ids.filter((value) => value !== id),
                            );
                          }}
                        />
                        <Label htmlFor={`app-${id}`}>
                          Selected app: {id} (details unavailable)
                        </Label>
                      </div>
                    ))}
                    {developerApps.map((app) => {
                      const selected = form.watch("developer_app_ids") ?? [];
                      const checked = selected.includes(app.id);
                      return (
                        <div
                          key={app.id}
                          className="flex items-center gap-2 rounded-lg border border-border p-2"
                        >
                          <Checkbox
                            id={`app-${app.id}`}
                            checked={checked}
                            disabled={!app.is_active && !checked}
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
                            {!app.is_active ? " (inactive)" : ""}
                          </Label>
                          <Badge
                            variant="secondary"
                            className="ml-auto text-xs"
                          >
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

                <FormField
                  control={form.control}
                  name="ssh_auth_mode"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>SSH Authentication Mode</FormLabel>
                      <Select
                        value={field.value}
                        onValueChange={field.onChange}
                      >
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          {SSH_AUTH_MODES.map((mode) => (
                            <SelectItem key={mode} value={mode}>
                              {SSH_AUTH_MODE_LABELS[mode]}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <div className="grid gap-4 sm:grid-cols-2">
                  {form.watch("ssh_auth_mode") === "cert" && (
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
                  )}
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
                          Comma-separated SSH usernames allowed for this
                          service.
                        </p>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                </div>
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
                  errors={flattenRowErrors(
                    form.formState.errors.ws_frame_injections,
                  )}
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
                            them. Sensitive is a UI redaction flag only — values
                            are stored plaintext in v1, so do not place real
                            secrets here (use the service auth method instead).
                          </p>
                        </div>
                        <FormField
                          control={form.control}
                          name="default_request_headers"
                          render={({ field, formState }) => (
                            // Nested array errors (numeric keys on the array
                            // field) need per-row rendering or blank-name /
                            // invalid-value submits block silently (NyxID#356
                            // code review P3).
                            <FormItem>
                              <FormControl>
                                <DefaultHeadersEditor
                                  value={field.value ?? []}
                                  onChange={(next) =>
                                    field.onChange(next.map((h) => ({ ...h })))
                                  }
                                  errors={flattenRowFieldErrors(
                                    formState.errors.default_request_headers,
                                  )}
                                />
                              </FormControl>
                              <FormMessage />
                            </FormItem>
                          )}
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
                              [
                                "supports_direct_downstream_auth",
                                "Direct Downstream Auth",
                              ],
                              [
                                "supports_authoring_via_nyx",
                                "Authoring via NyxID",
                              ],
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
                                onCheckedChange={(v) => form.setValue(key, v)}
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
                          checked={form.watch("forward_access_token") ?? false}
                          onCheckedChange={(v) =>
                            form.setValue("forward_access_token", v)
                          }
                        />
                      </div>
                    </div>

                    <Separator className="my-2" />
                    {user?.is_admin && (
                      <PlatformServiceFields form={form} service={service} />
                    )}
                    <div className="space-y-4">
                      <div className="space-y-1">
                        <h3 className="text-[13px] font-semibold">Billing</h3>
                        <p className="text-xs text-muted-foreground">
                          Services are free by default: usage is metered for
                          observability but never charged. Enable platform
                          billing to reserve and charge wallet credits for
                          requests to this service at the plan&apos;s platform
                          rates.
                        </p>
                      </div>

                      <div className="flex items-center justify-between rounded-lg border border-border p-3">
                        <Label
                          htmlFor="platform-billable"
                          className="text-[12px] font-normal"
                        >
                          Charge wallet credits (platform billing)
                        </Label>
                        <Switch
                          id="platform-billable"
                          checked={form.watch("platform_billable") ?? false}
                          onCheckedChange={(v) =>
                            form.setValue("platform_billable", v)
                          }
                        />
                      </div>

                      <div className="flex items-center justify-between gap-4 rounded-lg border border-border p-3">
                        <div className="space-y-1">
                          <Label
                            htmlFor="platform-charge-nyxid-credentials-only"
                            className="text-[12px] font-normal"
                          >
                            Charge only NyxID-provided credentials
                          </Label>
                          <p className="text-xs text-muted-foreground">
                            Charge NyxID master keys and shared OAuth apps.
                            Users bringing their own credentials are metered for
                            observability without platform charges.
                          </p>
                        </div>
                        <Switch
                          id="platform-charge-nyxid-credentials-only"
                          checked={
                            form.watch(
                              "platform_charge_nyxid_credentials_only",
                            ) ?? false
                          }
                          onCheckedChange={(v) =>
                            form.setValue(
                              "platform_charge_nyxid_credentials_only",
                              v,
                            )
                          }
                        />
                      </div>

                      <div className="grid gap-4 sm:grid-cols-2">
                        <div className="space-y-2">
                          <Label className="text-[12px] font-normal">
                            Charge by
                          </Label>
                          <Select
                            value={form.watch("platform_metric") ?? "auto"}
                            onValueChange={(v) =>
                              form.setValue(
                                "platform_metric",
                                v as UpdateServiceFormData["platform_metric"],
                              )
                            }
                          >
                            <SelectTrigger>
                              <SelectValue placeholder="Auto (derived from service)" />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem value="auto">
                                Auto (derived from service)
                              </SelectItem>
                              <SelectItem value="tokens">Tokens</SelectItem>
                              <SelectItem value="requests">Requests</SelectItem>
                              <SelectItem value="bytes">Bytes</SelectItem>
                            </SelectContent>
                          </Select>
                        </div>

                        {user?.is_admin && (
                          <FormField
                            control={form.control}
                            name="platform_price"
                            render={({ field }) => (
                              <FormItem>
                                <div className="flex min-h-5 items-center justify-between gap-2">
                                  <FormLabel>Credits per unit</FormLabel>
                                  {service.billing?.platform_pricing && (
                                    <Badge
                                      variant={
                                        service.billing.platform_pricing
                                          .sync_status === "synced"
                                          ? "success"
                                          : service.billing.platform_pricing
                                                .sync_status === "failed"
                                            ? "destructive"
                                            : "warning"
                                      }
                                    >
                                      {service.billing.platform_pricing
                                        .sync_status ?? "pending"}
                                    </Badge>
                                  )}
                                </div>
                                <FormControl>
                                  <Input
                                    {...field}
                                    inputMode="decimal"
                                    placeholder="Uses Lago plan rate"
                                  />
                                </FormControl>
                                <FormMessage />
                              </FormItem>
                            )}
                          />
                        )}
                      </div>
                      {service.billing?.platform_pricing?.sync_error && (
                        <p className="text-xs text-destructive">
                          {service.billing.platform_pricing.sync_error}
                        </p>
                      )}
                      <p className="text-xs text-muted-foreground">
                        Auto meters tokens for llm- services, bytes for SSH and
                        WebSocket connections, and requests otherwise. An empty
                        price keeps the Lago-authored plan rate.
                      </p>
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
                                Available scopes:{" "}
                                {DELEGATION_TOKEN_SCOPES.join(", ")}
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

            <FormSubmitErrors className="pt-2 text-right" />
            <div className="flex items-center justify-end gap-3 pt-4">
              <Button
                variant="primary"
                type="submit"
                isLoading={updateMutation.isPending}
                disabled={stale || !form.formState.isDirty}
              >
                Save Changes
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
