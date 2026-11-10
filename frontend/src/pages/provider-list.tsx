import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useProviders, useCreateProvider } from "@/hooks/use-providers";
import {
  createProviderSchema,
  type CreateProviderFormData,
  PROVIDER_TYPES,
} from "@/schemas/providers";
import {
  buildCreateProviderPayload,
  getProviderTypeFieldResets,
} from "./provider-list.helpers";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { ServiceIcon } from "@/components/service-icon";
import { DishAntennaIcon } from "@/components/icons/empty-state";
import { toast } from "sonner";

const PROVIDER_TYPE_LABELS: Readonly<Record<string, string>> = {
  get oauth2() { return translateUi("OAuth 2.0"); },
  get api_key() { return translateUi("API Key"); },
  get device_code() { return translateUi("Device Code"); },
  get telegram_widget() { return translateUi("Telegram Widget"); },
};

export function ProviderListPage() {
  const { t } = useUiTranslation();
  const { data: providers, isLoading } = useProviders();
  const createMutation = useCreateProvider();
  const navigate = useNavigate();
  const [createOpen, setCreateOpen] = useState(false);

  const form = useAppForm<CreateProviderFormData>({
    resolver: zodResolver(createProviderSchema),
    defaultValues: {
      name: "",
      slug: "",
      description: "",
      provider_type: "oauth2",
      credential_mode: "admin",
      authorization_url: "",
      token_url: "",
      revocation_url: "",
      default_scopes: "",
      client_id: "",
      client_secret: "",
      client_id_param_name: "",
      supports_pkce: true,
      device_code_url: "",
      device_token_url: "",
      hosted_callback_url: "",
      api_key_instructions: "",
      api_key_url: "",
      icon_url: "",
      documentation_url: "",
    },
  });

  const watchedProviderType = useWatch({
    control: form.control,
    name: "provider_type",
  });
  const watchedCredentialMode = useWatch({
    control: form.control,
    name: "credential_mode",
  });

  async function onSubmit(data: CreateProviderFormData) {
    try {
      const created = await createMutation.mutateAsync(
        buildCreateProviderPayload(data) as Parameters<
          typeof createMutation.mutateAsync
        >[0],
      );
      toast.success(translateUi("Provider created successfully"));
      setCreateOpen(false);
      form.reset();
      void navigate({ to: "/providers/$providerId", params: { providerId: created.id } });
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error(translateUi("Failed to create provider"));
      }
    }
  }

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="text-28 font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>{t("Manage Providers")}</h2>
          <p className="text-12 text-muted-foreground">{t("Create and manage OAuth, Telegram, device code, and API key providers.")}</p>
        </div>
        <AddCtaButton label={t("Add Provider")} onClick={() => setCreateOpen(true)} />
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t("Add Provider")}</DialogTitle>
              <DialogDescription>{t("Register a new external provider for user connections.")}</DialogDescription>
            </DialogHeader>

            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onSubmit)}
                className="space-y-4"
              >
                {form.formState.errors.root && (
                  <div className="rounded-lg bg-destructive/10 p-3 text-12 text-destructive">
                    {translateUi(form.formState.errors.root.message ?? "")}
                  </div>
                )}

                <FormField
                  control={form.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Name")}</FormLabel>
                      <FormControl>
                        <Input placeholder={t("GitHub")} {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="slug"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Slug")}</FormLabel>
                      <FormControl>
                        <Input placeholder={t("github")} {...field} />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">{t("Lowercase letters, digits, and hyphens only.")}</p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="description"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Description")}</FormLabel>
                      <FormControl>
                        <textarea
                          className="flex min-h-[60px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-12 placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-input-focus disabled:cursor-not-allowed disabled:opacity-50"
                          placeholder={t("Optional description")}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="provider_type"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Provider Type")}</FormLabel>
                      <Select
                        value={field.value}
                        onValueChange={(nextValue) => {
                          const nextType =
                            nextValue as CreateProviderFormData["provider_type"];
                          const resets = getProviderTypeFieldResets(
                            field.value,
                            nextType,
                          );

                          field.onChange(nextType);

                          const resetKeys = Object.keys(resets) as Array<
                            keyof typeof resets
                          >;
                          for (const key of resetKeys) {
                            form.setValue(
                              key as keyof CreateProviderFormData,
                              resets[key] as never,
                              {
                                shouldDirty: false,
                                shouldValidate: false,
                                shouldTouch: false,
                              },
                            );
                          }

                          if (resetKeys.length > 0) {
                            form.clearErrors(
                              resetKeys as Array<keyof CreateProviderFormData>,
                            );
                          }
                        }}
                      >
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue placeholder={t("Select provider type")} />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          {PROVIDER_TYPES.map((type) => (
                            <SelectItem key={type} value={type}>
                              {PROVIDER_TYPE_LABELS[type] ?? type}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                {(watchedProviderType === "oauth2" ||
                  watchedProviderType === "device_code") && (
                  <FormField
                    control={form.control}
                    name="credential_mode"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>{t("Credential Mode")}</FormLabel>
                        <Select
                          value={field.value ?? "admin"}
                          onValueChange={field.onChange}
                        >
                          <FormControl>
                            <SelectTrigger>
                              <SelectValue />
                            </SelectTrigger>
                          </FormControl>
                          <SelectContent>
                            <SelectItem value="admin">{t("Admin Only")}</SelectItem>
                            <SelectItem value="user">{t("User Provided")}</SelectItem>
                            <SelectItem value="both">{t("Admin or User")}</SelectItem>
                          </SelectContent>
                        </Select>
                        <p className="text-xs text-muted-foreground">{t("Choose whether users rely on admin-managed OAuth apps, bring their own, or can use either.")}</p>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                )}

                <FormField
                  control={form.control}
                  name="icon_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Icon URL")}</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="https://example.com/icon.svg"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="documentation_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Documentation URL")}</FormLabel>
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

                {watchedProviderType === "oauth2" && (
                  <>
                    <Separator />
                    <h4 className="text-13 font-semibold">{t("OAuth 2.0 Configuration")}</h4>

                    <FormField
                      control={form.control}
                      name="authorization_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Authorization URL")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://provider.com/oauth/authorize"
                              {...field}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="token_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Token URL")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://provider.com/oauth/token"
                              {...field}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="revocation_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Revocation URL")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://provider.com/oauth/revoke"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("Optional. Used to revoke tokens on disconnect.")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="default_scopes"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Default Scopes")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder={t("read, write, user:email")}
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("Comma-separated list of scopes.")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="client_id"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Client ID")}</FormLabel>
                          <FormControl>
                            <Input placeholder={t("OAuth client ID")} {...field} />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            {watchedCredentialMode === "admin"
                              ? t("Required in admin mode.")
                              : t("Optional. Leave blank to require users to supply their own OAuth app credentials.")}
                          </p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="client_secret"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Client Secret")}</FormLabel>
                          <FormControl>
                            <Input
                              type="password"
                              placeholder={t("OAuth client secret")}
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            {watchedCredentialMode === "admin"
                              ? t("Required for OAuth 2.0 admin credentials.")
                              : t("Optional. Only set this when configuring an admin fallback OAuth app.")}
                          </p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="supports_pkce"
                      render={({ field }) => (
                        <FormItem className="flex items-center justify-between rounded-lg border p-3">
                          <div className="space-y-0.5">
                            <FormLabel>{t("Supports PKCE")}</FormLabel>
                            <p className="text-xs text-muted-foreground">{t("Enable Proof Key for Code Exchange.")}</p>
                          </div>
                          <FormControl>
                            <Switch
                              checked={field.value ?? true}
                              onCheckedChange={field.onChange}
                            />
                          </FormControl>
                        </FormItem>
                      )}
                    />
                  </>
                )}

                {watchedProviderType === "device_code" && (
                  <>
                    <Separator />
                    <h4 className="text-13 font-semibold">{t("Device Code Configuration (RFC 8628)")}</h4>

                    <FormField
                      control={form.control}
                      name="device_code_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Device Code URL")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://auth.openai.com/deviceauth/usercode"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("Endpoint to request a device code (RFC 8628 step 1).")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="device_token_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Device Token URL")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://auth.openai.com/deviceauth/token"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("Endpoint to poll for token (RFC 8628 step 3).")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="authorization_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Authorization URL (fallback)")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://auth.openai.com/oauth/authorize"
                              {...field}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="token_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Token URL (fallback)")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://auth.openai.com/oauth/token"
                              {...field}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="default_scopes"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Default Scopes")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder={t("openid, profile, offline_access")}
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("Comma-separated list of scopes.")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="client_id"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Client ID")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder={t("app_EMoamEEZ73f0CkXaXp7hrann")}
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            {watchedCredentialMode === "admin"
                              ? t("Required in admin mode.")
                              : t("Optional. Needed only when configuring an admin fallback client.")}
                          </p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="client_secret"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Client Secret (optional)")}</FormLabel>
                          <FormControl>
                            <Input
                              type="password"
                              placeholder={t("OAuth client secret (if required)")}
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("Optional for public OAuth clients.")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </>
                )}

                {watchedProviderType === "telegram_widget" && (
                  <>
                    <Separator />
                    <h4 className="text-13 font-semibold">{t("Telegram Widget Configuration")}</h4>

                    <FormField
                      control={form.control}
                      name="client_id_param_name"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Bot Username")}</FormLabel>
                          <FormControl>
                            <Input placeholder={t("NyxIdBot")} {...field} />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("Enter the BotFather username. A leading")}<span> @</span>{t(" is optional.")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="client_secret"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("Bot Token")}</FormLabel>
                          <FormControl>
                            <Input
                              type="password"
                              placeholder={t("123456:ABC-DEF1234567890")}
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("BotFather token used to verify Telegram Login Widget callbacks.")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </>
                )}

                {watchedProviderType === "api_key" && (
                  <>
                    <Separator />
                    <h4 className="text-13 font-semibold">{t("API Key Configuration")}</h4>

                    <FormField
                      control={form.control}
                      name="api_key_instructions"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("API Key Instructions")}</FormLabel>
                          <FormControl>
                            <textarea
                              className="flex min-h-[60px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-12 placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-input-focus disabled:cursor-not-allowed disabled:opacity-50"
                              placeholder={t("Instructions for users to obtain an API key")}
                              {...field}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="api_key_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>{t("API Key URL")}</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://provider.com/api-keys"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">{t("Link where users can generate an API key.")}</p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </>
                )}

                <DialogFooter>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setCreateOpen(false)}
                  >{t("Cancel")}</Button>
                  <Button variant="primary" type="submit" isLoading={createMutation.isPending} disabled={!form.formState.isValid || createMutation.isPending}>{t("Create Provider")}</Button>
                </DialogFooter>
              </form>
            </Form>
          </DialogContent>
        </Dialog>
      </div>

      {isLoading ? (
        <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={`prov-skel-${String(i)}`} className="h-36 w-full" />
          ))}
        </div>
      ) : !providers || providers.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-1 py-12 text-center">
          <DishAntennaIcon className="h-64 w-64 text-muted-foreground" />
          <div className="space-y-1">
            <p className="text-12 font-medium text-muted-foreground">{t("No Providers")}</p>
            <p className="text-xs text-muted-foreground">{t("Add a provider to get started.")}</p>
          </div>
        </div>
      ) : (
        <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
          {providers.map((provider) => (
            <Card
              key={provider.id}
              className="cursor-pointer transition-colors duration-300 hover:border-hairline-strong"
              onClick={() =>
                void navigate({
                  to: "/providers/$providerId",
                  params: { providerId: provider.id },
                })
              }
            >
              <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-3">
                <div className="flex items-center gap-3">
                  <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary/10">
                    <ServiceIcon
                      slug={provider.slug}
                      iconUrl={provider.icon_url}
                      size="sm"
                    />
                  </div>
                  <div>
                    <CardTitle>{provider.name}</CardTitle>
                    <CardDescription className="text-xs">
                      {provider.slug}
                    </CardDescription>
                  </div>
                </div>
              </CardHeader>
              <CardContent>
                <div className="flex flex-col gap-2">
                  {provider.description && (
                    <p className="text-xs text-muted-foreground line-clamp-2">
                      {provider.description}
                    </p>
                  )}
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <Badge variant="secondary">
                        {PROVIDER_TYPE_LABELS[provider.provider_type] ??
                          provider.provider_type}
                      </Badge>
                      <Badge
                        variant={provider.is_active ? "success" : "secondary"}
                      >
                        {provider.is_active ? t("Active") : t("Inactive")}
                      </Badge>
                    </div>
                    <span className="text-xs text-muted-foreground">
                      {formatDate(provider.created_at)}
                    </span>
                  </div>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}
