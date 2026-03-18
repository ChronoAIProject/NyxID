import { useEffect } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useService, useUpdateService } from "@/hooks/use-services";
import {
  updateServiceSchema,
  type UpdateServiceFormData,
} from "@/schemas/services";
import {
  getAuthTypeLabel,
  SERVICE_CATEGORY_LABELS,
  SERVICE_TYPE_LABELS,
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
import { Switch } from "@/components/ui/switch";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { AlertCircle } from "lucide-react";
import { toast } from "sonner";

export function ServiceEditPage() {
  const { serviceId } = useParams({ strict: false }) as { serviceId: string };
  const navigate = useNavigate();
  const { data: service, isLoading, error } = useService(serviceId);
  const updateMutation = useUpdateService();
  const user = useAuthStore((s) => s.user);

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
      inject_delegation_token: false,
      delegation_token_scope: "",
      host: "",
      port: "22",
      certificate_auth_enabled: false,
      certificate_ttl_minutes: "30",
      allowed_principals: "",
    },
  });

  useEffect(() => {
    if (service) {
      form.reset({
        service_type: service.service_type === "ssh" ? "ssh" : "http",
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
        inject_delegation_token: service.inject_delegation_token ?? false,
        delegation_token_scope: service.delegation_token_scope || "llm:proxy",
        host: service.ssh_config?.host ?? "",
        port: service.ssh_config ? String(service.ssh_config.port) : "22",
        certificate_auth_enabled:
          service.ssh_config?.certificate_auth_enabled ?? false,
        certificate_ttl_minutes: service.ssh_config
          ? String(service.ssh_config.certificate_ttl_minutes)
          : "30",
        allowed_principals:
          service.ssh_config?.allowed_principals.join(", ") ?? "",
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [service]);

  async function onSubmit(data: UpdateServiceFormData) {
    if (!service) return;
    try {
      await updateMutation.mutateAsync({
        serviceId: service.id,
        data:
          service.service_type === "ssh"
            ? {
                name: data.name,
                description: data.description || "",
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
                base_url: data.base_url || "",
                openapi_spec_url: data.openapi_spec_url || "",
                asyncapi_spec_url: data.asyncapi_spec_url || "",
                identity_propagation_mode: data.identity_propagation_mode,
                identity_include_user_id: data.identity_include_user_id,
                identity_include_email: data.identity_include_email,
                identity_include_name: data.identity_include_name,
                identity_jwt_audience: data.identity_jwt_audience || "",
                inject_delegation_token: data.inject_delegation_token,
                delegation_token_scope: data.delegation_token_scope || "",
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
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">
          Service not found
        </h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The service you are trying to edit does not exist or has been deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/services" })}
        >
          Back to Services
        </Button>
      </div>
    );
  }

  const isSshService = service.service_type === "ssh";

  return (
    <div className="space-y-8">
      <PageHeader
        breadcrumbs={[
          { label: "Services", to: "/services" },
          {
            label: service.name,
            to: `/services/${serviceId}`,
          },
          { label: "Edit" },
        ]}
        title={`Edit ${service.name}`}
      />

      <div className="max-w-2xl">
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {form.formState.errors.root && (
              <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
                {form.formState.errors.root.message}
              </div>
            )}

            <div className="flex flex-wrap gap-2">
              <Badge variant="secondary">
                {SERVICE_TYPE_LABELS[service.service_type] ??
                  service.service_type}
              </Badge>
              <Badge variant="outline">
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
                      className="flex min-h-[80px] w-full rounded-[10px] border border-input bg-transparent px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                      placeholder="Optional description"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

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

                <div className="flex items-center justify-between rounded-[10px] border border-border p-3">
                  <Label
                    htmlFor="edit-ssh-cert-auth"
                    className="text-sm font-normal"
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

                <div>
                  <p className="mb-1 text-sm font-medium">Auth Type</p>
                  <Badge variant="secondary">{getAuthTypeLabel(service)}</Badge>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Auth type cannot be changed after creation.
                  </p>
                </div>

                {user && (
                  <>
                    <Separator className="my-2" />
                    <div className="space-y-2">
                      <h3 className="text-sm font-semibold">
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
                        <h3 className="text-sm font-semibold">
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

                      <div className="flex items-center justify-between rounded-[10px] border border-border p-3">
                        <Label
                          htmlFor="inject-delegation-token"
                          className="text-sm font-normal"
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

            <div className="flex items-center gap-3 pt-4">
              <Button type="submit" isLoading={updateMutation.isPending}>
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
