import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import type { ProviderConfig } from "@/types/api";
import { PlatformServiceFields } from "./platform-service-fields";
import { useNavigate } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { useCreateService } from "@/hooks/use-services";
import {
  createServiceSchema,
  type CreateServiceFormData,
  AUTH_TYPES,
  SERVICE_CATEGORIES,
  SERVICE_TYPES,
  VISIBILITY_OPTIONS,
} from "@/schemas/services";
import {
  AUTH_TYPE_LABELS,
  SERVICE_CATEGORY_LABELS,
  SERVICE_TYPE_LABELS,
  VISIBILITY_LABELS,
} from "@/lib/constants";
import { parseAllowedPrincipals } from "@/lib/ssh";
import { ApiError } from "@/lib/api-client";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  useAppForm,
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
import { toast } from "sonner";

export function CreateServiceDialog({
  open,
  onOpenChange,
  provider,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly provider?: Pick<ProviderConfig, "id" | "name">;
}) {
  useUiTranslation();
  const navigate = useNavigate();
  const createMutation = useCreateService();
  const form = useAppForm<CreateServiceFormData>({
    resolver: zodResolver(createServiceSchema),
    defaultValues: {
      name: "",
      description: "",
      service_type: "http",
      base_url: "",
      auth_type: "api_key",
      auth_key_name: "",
      auth_scheme: "",
      credential: "",
      platform_key: {
        enabled: false,
        audience: "restricted",
        allowed_owner_ids: [],
      },
      service_category: "connection",
      host: "",
      port: "22",
      certificate_auth_enabled: false,
      certificate_ttl_minutes: "30",
      allowed_principals: "",
    },
  });

  const serviceType = form.watch("service_type");
  const authType = form.watch("auth_type");
  const certificateAuthEnabled =
    form.watch("certificate_auth_enabled") ?? false;

  async function onSubmit(data: CreateServiceFormData) {
    try {
      const created =
        data.service_type === "ssh"
          ? await createMutation.mutateAsync({
              name: data.name,
              description: data.description || undefined,
              service_type: "ssh",
              visibility: data.visibility ?? "private",
              service_category: data.service_category ?? "internal",
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
            })
          : await createMutation.mutateAsync({
              name: data.name,
              description: data.description || undefined,
              service_type: "http",
              visibility: data.visibility ?? "public",
              base_url: data.base_url ?? "",
              auth_type: data.auth_type ?? "api_key",
              auth_key_name: data.auth_key_name || undefined,
              auth_scheme: data.auth_scheme || undefined,
              provider_config_id: provider?.id,
              inference: data.inference,
              platform_key:
                data.auth_type === "none" || data.auth_type === "oidc"
                  ? undefined
                  : data.platform_key,
              proxy_operation_policy: data.proxy_operation_policy,
              billing: {
                byok_pricing: data.byok_pricing,
                platform_key_pricing: data.platform_key_pricing,
              },
              credential: data.credential?.trim() ? data.credential : undefined,
              service_category: data.service_category ?? "connection",
            });

      toast.success(translateUi("Service created successfully"));
      onOpenChange(false);
      form.reset();
      void navigate({
        to: "/services/$serviceId",
        params: { serviceId: created.id },
      });
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError ? error.message : "Failed to create service"),
      );
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {provider
              ? translateUi("Create service for {{value1}}", { value1: provider.name })
              : translateUi("Create Service")}
          </DialogTitle>
          <DialogDescription>{translateUi("Pick the service type first, then configure the fields NyxID should manage.")}</DialogDescription>
        </DialogHeader>

        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            <FormField
              control={form.control}
              name="service_type"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Service Type")}</FormLabel>
                  <Select
                    value={field.value}
                    onValueChange={(value) => {
                      field.onChange(value);
                      form.clearErrors();
                      if (value === "ssh") {
                        form.setValue("service_category", "internal");
                        form.setValue("visibility", "private");
                      } else {
                        form.setValue("service_category", "connection");
                        form.setValue("visibility", "public");
                      }
                    }}
                  >
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue placeholder={translateUi("Select service type")} />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      {SERVICE_TYPES.filter(
                        (type) => !provider || type === "http",
                      ).map((type) => (
                        <SelectItem key={type} value={type}>
                          {SERVICE_TYPE_LABELS[type] ?? type}
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
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Service Name")}</FormLabel>
                  <FormControl>
                    <Input placeholder={translateUi("My Service")} {...field} />
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
                  <FormLabel>{translateUi("Description")}</FormLabel>
                  <FormControl>
                    <textarea
                      className="flex min-h-[60px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-14 placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-input-focus disabled:cursor-not-allowed disabled:opacity-50"
                      placeholder={translateUi("Optional description")}
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            {serviceType === "ssh" ? (
              <>
                <div className="grid gap-4 sm:grid-cols-2">
                  <FormField
                    control={form.control}
                    name="host"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{translateUi("SSH Host")}</FormLabel>
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
                        <FormLabel>{translateUi("SSH Port")}</FormLabel>
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
                    htmlFor="create-ssh-cert-auth"
                  >{translateUi("Enable short-lived SSH certificates")}</Label>
                  <Switch
                    id="create-ssh-cert-auth"
                    checked={certificateAuthEnabled}
                    onCheckedChange={(checked) =>
                      form.setValue("certificate_auth_enabled", checked)
                    }
                  />
                </div>

                {certificateAuthEnabled && (
                  <div className="grid gap-4 sm:grid-cols-2">
                    <FormField
                      control={form.control}
                      name="certificate_ttl_minutes"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{translateUi("Certificate TTL (minutes)")}</FormLabel>
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
                          <FormLabel>{translateUi("Allowed Principals")}</FormLabel>
                          <FormControl>
                            <Input placeholder={translateUi("ubuntu, deploy")} {...field} />
                          </FormControl>
                          <p className="text-13 text-muted-foreground">{translateUi("Comma-separated SSH usernames NyxID is allowed to sign.")}</p>
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
                      <FormLabel>{translateUi("Base URL")}</FormLabel>
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
                  name="auth_type"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Auth Type")}</FormLabel>
                      <Select
                        value={field.value}
                        onValueChange={(value) => {
                          field.onChange(value);
                          if (value === "oidc") {
                            form.setValue("service_category", "provider");
                          } else if (value === "none") {
                            form.setValue("service_category", "internal");
                          } else if (
                            form.getValues("service_category") === "provider" ||
                            form.getValues("service_category") === "internal"
                          ) {
                            form.setValue("service_category", "connection");
                          }
                          // Clear auth_key_name when switching away from
                          // body auth so stale values don't carry over.
                          if (value !== "body") {
                            form.setValue("auth_key_name", "");
                          }
                          if (!["bearer", "oauth2"].includes(value)) {
                            form.setValue("auth_scheme", "");
                          }
                          if (value === "oidc" || value === "none") {
                            form.setValue("credential", "");
                          }
                        }}
                      >
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue placeholder={translateUi("Select auth type")} />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          {AUTH_TYPES.filter(
                            (type) =>
                              !provider || !["oidc", "none"].includes(type),
                          ).map((type) => (
                            <SelectItem key={type} value={type}>
                              {AUTH_TYPE_LABELS[type] ?? type}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      {authType === "body" && (
                        <p className="text-13 text-muted-foreground">{translateUi("NyxID will merge")}{" "}
                          <code>{`{<field>: <credential>}`}</code>{translateUi(" into the JSON request body. Used for Lark/Feishu tenant token exchange where ")}<code>app_secret</code>{translateUi(" must travel in the body.")}</p>
                      )}
                      {authType === "bot_bearer" && (
                        <p className="text-13 text-muted-foreground">{translateUi("Sends")}{" "}
                          <code>Authorization: Bot &lt;credential&gt;</code>{" "}{translateUi("(Discord bot token format -- note the ")}<code>Bot</code>{" "}{translateUi("prefix, not ")}<code>Bearer</code>).
                        </p>
                      )}
                      <FormMessage />
                    </FormItem>
                  )}
                />

                {(authType === "bearer" || authType === "oauth2") && (
                  <FormField
                    control={form.control}
                    name="auth_scheme"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{translateUi("Authorization scheme")}</FormLabel>
                        <FormControl>
                          <Input
                            {...field}
                            value={field.value ?? ""}
                            placeholder={translateUi("Bearer")}
                            maxLength={64}
                          />
                        </FormControl>
                        <p className="text-14 text-muted-foreground">{translateUi("Leave blank for Bearer. Use the scheme documented by the vendor.")}</p>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                )}

                {authType === "body" && (
                  <FormField
                    control={form.control}
                    name="auth_key_name"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{translateUi("Body Field Name")}</FormLabel>
                        <FormControl>
                          <Input
                            placeholder="app_secret"
                            {...field}
                            value={field.value ?? ""}
                          />
                        </FormControl>
                        <p className="text-13 text-muted-foreground">{translateUi("The JSON field where the credential will be injected. For Lark/Feishu use ")}<code>app_secret</code>.
                        </p>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                )}

                <PlatformServiceFields
                  credentialSupported={
                    authType !== "oidc" && authType !== "none"
                  }
                />

                {form.watch("auth_type") !== "oidc" &&
                  form.watch("auth_type") !== "none" && (
                    <FormField
                      control={form.control}
                      name="service_category"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{translateUi("Service Category")}</FormLabel>
                          <Select
                            value={field.value ?? "connection"}
                            onValueChange={field.onChange}
                          >
                            <FormControl>
                              <SelectTrigger>
                                <SelectValue placeholder={translateUi("Select category")} />
                              </SelectTrigger>
                            </FormControl>
                            <SelectContent>
                              {SERVICE_CATEGORIES.filter(
                                (cat) => cat !== "provider",
                              ).map((cat) => (
                                <SelectItem key={cat} value={cat}>
                                  {SERVICE_CATEGORY_LABELS[cat] ?? cat}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  )}
              </>
            )}

            <FormField
              control={form.control}
              name="visibility"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{translateUi("Visibility")}</FormLabel>
                  <Select
                    value={
                      field.value ??
                      (serviceType === "ssh" ? "private" : "public")
                    }
                    onValueChange={field.onChange}
                  >
                    <FormControl>
                      <SelectTrigger>
                        <SelectValue placeholder={translateUi("Select visibility")} />
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
                  <p className="text-13 text-muted-foreground">{translateUi("Private services are only visible to you.")}</p>
                  <FormMessage />
                </FormItem>
              )}
            />

            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
              >{translateUi("Cancel")}</Button>
              <Button
                variant="primary"
                type="submit"
                isLoading={createMutation.isPending}
              >{translateUi("Create service")}</Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
