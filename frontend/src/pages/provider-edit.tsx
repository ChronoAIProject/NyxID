import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { ProviderServices } from "@/components/providers/provider-services";
import { useState } from "react";
import { canAdminWrite, type ProviderConfig } from "@/types/api";
import { useAuthStore } from "@/stores/auth-store";
import { changedFields, describeChanges, sameValue } from "@/lib/form-changes";
import {
  useChangeReview,
  useEditorMounted,
} from "@/components/shared/change-review-dialog";
import { StaleFormNotice } from "@/components/shared/stale-form-notice";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useProvider, useUpdateProvider } from "@/hooks/use-providers";
import {
  updateProviderSchema,
  type UpdateProviderFormData,
} from "@/schemas/providers";
import { ApiError } from "@/lib/api-client";
import { PageHeader } from "@/components/shared/page-header";
import { useBreadcrumbLabel } from "@/components/layout/breadcrumb-context";
import { Separator } from "@/components/ui/separator";
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
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { ErrorBanner } from "@/components/shared/error-banner";
import { toast } from "sonner";
import {
  PROVIDER_TYPE_LABELS,
  providerFormPayload,
  providerFormValues,
} from "./provider-edit.helpers";

export function ProviderEditPage() {
  const { t } = useUiTranslation();
  const { providerId } = useParams({ strict: false }) as { providerId: string };
  const { data: provider, isLoading, error, refetch } = useProvider(providerId);
  useBreadcrumbLabel(provider?.name);
  if (isLoading && !provider) return <Skeleton className="h-96 w-full" />;
  if (!provider)
    return (
      <ErrorBanner
        message={
          error instanceof ApiError ? error.message : t("Unable to load provider")
        }
        onRetry={refetch}
      />
    );
  return <ProviderEditForm key={providerId} source={provider} />;
}

function ProviderEditForm({ source }: { readonly source: ProviderConfig }) {
  const { t } = useUiTranslation();
  const [provider, setProvider] = useState(source);
  const providerId = provider.id;
  const isMounted = useEditorMounted();
  const navigate = useNavigate();
  const updateMutation = useUpdateProvider(providerId);
  const isAdmin = canAdminWrite(useAuthStore((state) => state.user));

  const form = useAppForm<UpdateProviderFormData>({
    resolver: zodResolver(updateProviderSchema),
    defaultValues: providerFormValues(provider),
  });
  const stale =
    !sameValue(providerFormValues(provider), providerFormValues(source)) ||
    !sameValue(provider.revocation, source.revocation) ||
    ((!!form.watch("client_id")?.trim() ||
      !!form.watch("client_secret")?.trim()) &&
      (provider.updated_at !== source.updated_at ||
        provider.has_client_id !== source.has_client_id ||
        provider.has_client_secret !== source.has_client_secret));
  type ReviewedProvider = {
    providerId: string;
    data: Parameters<typeof updateMutation.mutateAsync>[0];
  };
  const review = useChangeReview<ReviewedProvider>(
    saveChanges,
    stale,
    providerId,
  );

  const watchedProviderType = useWatch({
    control: form.control,
    name: "provider_type",
  });

  function onSubmit(data: UpdateProviderFormData) {
    const before = providerFormPayload(providerFormValues(provider));
    const patch = changedFields(before, providerFormPayload(data));
    for (const field of ["authorization_url", "token_url"] as const) {
      if (patch[field] === "") {
        form.setError(field, {
          message: "This endpoint cannot be cleared; enter a valid URL.",
        });
        return;
      }
    }
    const payload: Parameters<typeof updateMutation.mutateAsync>[0] = {
      ...patch,
    };
    if (patch.revocation_url && provider.revocation) {
      delete payload.revocation_url;
      payload.revocation = {
        ...provider.revocation,
        url: patch.revocation_url,
      };
    }
    if (patch.revocation_url === "") {
      delete payload.revocation_url;
      payload.revocation = null;
    }
    review.review(
      { providerId, data: payload },
      describeChanges(before, patch, {
        secretFields: ["client_id", "client_secret"],
      }),
    );
  }

  async function saveChanges({ providerId: targetId, data }: ReviewedProvider) {
    if (targetId !== providerId) return;
    try {
      await updateMutation.mutateAsync(data);
      if (!isMounted()) return;
      toast.success(translateUi("Provider updated"));
      void navigate({
        to: "/providers/$providerId",
        params: { providerId },
      });
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error(translateUi("Failed to update provider"));
      }
      throw err;
    }
  }

  const isOAuth = watchedProviderType === "oauth2";
  const isDeviceCode = watchedProviderType === "device_code";
  const isApiKey = watchedProviderType === "api_key";
  const isTelegram = watchedProviderType === "telegram_widget";

  return (
    <div className="space-y-8">
      <PageHeader title={t("Edit {{name}}", { name: provider.name })} />

      <ProviderServices provider={source} />
      {stale && (
        <StaleFormNotice
          onReload={() => {
            setProvider(source);
            form.reset(providerFormValues(source));
            review.cancel();
          }}
        />
      )}
      {review.dialog}
      <div className="max-w-2xl">
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
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
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <div>
              <p className="text-12 font-medium mb-1">{t("Slug")}</p>
              <Badge variant="secondary">{provider.slug}</Badge>
              <p className="text-xs text-muted-foreground mt-1">{t("Slug cannot be changed after creation.")}</p>
            </div>

            <div>
              <p className="text-12 font-medium mb-1">{t("Provider Type")}</p>
              <Badge variant="secondary">
                {PROVIDER_TYPE_LABELS[provider.provider_type] ??
                  provider.provider_type}
              </Badge>
              <p className="text-xs text-muted-foreground mt-1">{t("Provider type cannot be changed after creation.")}</p>
            </div>

            <FormField
              control={form.control}
              name="description"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>{t("Description")}</FormLabel>
                  <FormControl>
                    <textarea
                      className="flex min-h-[80px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-12 placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-input-focus disabled:cursor-not-allowed disabled:opacity-50"
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
              name="is_active"
              render={({ field }) => (
                <FormItem className="flex items-center justify-between rounded-lg border p-3">
                  <div className="space-y-0.5">
                    <FormLabel>{t("Active")}</FormLabel>
                    <p className="text-xs text-muted-foreground">{t("Inactive providers will not be available for user connections.")}</p>
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

            {(isOAuth || isDeviceCode) && (
              <FormField
                control={form.control}
                name="credential_mode"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t("Credential Mode")}</FormLabel>
                    <Select
                      value={field.value || "admin"}
                      onValueChange={field.onChange}
                    >
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder={t("Admin Only")} />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="admin">{t("Admin Only")}</SelectItem>
                        <SelectItem value="user">{t("User Provided")}</SelectItem>
                        <SelectItem value="both">{t("Admin or User")}</SelectItem>
                      </SelectContent>
                    </Select>
                    <p className="text-xs text-muted-foreground">{t("Controls whether admin-configured or user-provided OAuth credentials are used for connections.")}</p>
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
                    <Input placeholder="https://docs.example.com" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            {isOAuth && (
              <>
                <Separator className="my-2" />
                <h3 className="text-13 font-semibold">{t("OAuth 2.0 Configuration")}</h3>
                <p className="text-xs text-muted-foreground">{t("Saved endpoint URLs are shown below.")}</p>

                <FormField
                  control={form.control}
                  name="authorization_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Authorization URL")}</FormLabel>
                      <FormControl>
                        <Input placeholder="https://…" {...field} />
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
                        <Input placeholder="https://…" {...field} />
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
                        <Input placeholder="https://…" {...field} />
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
                      <FormLabel>{t("Client ID —")}{" "}
                        {provider.has_client_id
                          ? t("Configured")
                          : t("Not configured")}
                      </FormLabel>
                      <FormControl>
                        <Input
                          placeholder={t("Leave blank to keep current")}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="client_secret"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Client Secret —")}{" "}
                        {provider.has_client_secret
                          ? t("Configured")
                          : t("Not configured")}
                      </FormLabel>
                      <FormControl>
                        <Input
                          type="password"
                          placeholder={t("Leave blank to keep current")}
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">{t("Only fill in if you want to change the client secret.")}</p>
                      {!isAdmin && provider.has_client_secret && (
                        <p className="text-xs text-muted-foreground">{t("Catalog Managers: on a provider someone else created, URLs, credential mode and token settings can change only before anyone has connected, and only with the current client ID and secret re-entered here. Replacing the client requires an Admin.")}</p>
                      )}
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

            {isDeviceCode && (
              <>
                <Separator className="my-2" />
                <h3 className="text-13 font-semibold">{t("Device Code Configuration (RFC 8628)")}</h3>
                <p className="text-xs text-muted-foreground">{t("Saved endpoint URLs are shown below.")}</p>

                <FormField
                  control={form.control}
                  name="device_code_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("Device Code URL")}</FormLabel>
                      <FormControl>
                        <Input placeholder="https://…" {...field} />
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
                        <Input placeholder="https://…" {...field} />
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
                        <Input placeholder="https://…" {...field} />
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
                        <Input placeholder="https://…" {...field} />
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
                      <FormLabel>{t("Client ID —")}{" "}
                        {provider.has_client_id
                          ? t("Configured")
                          : t("Not configured")}
                      </FormLabel>
                      <FormControl>
                        <Input
                          placeholder={t("Leave blank to keep current")}
                          {...field}
                        />
                      </FormControl>
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
                          placeholder={t("Leave blank to keep current")}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </>
            )}

            {isTelegram && (
              <>
                <Separator className="my-2" />
                <h3 className="text-13 font-semibold">{t("Telegram Widget Configuration")}</h3>
                <p className="text-xs text-muted-foreground">{t("Leave the bot token blank to keep the current secret.")}</p>

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
                      <FormLabel>{t("Bot Token —")}{" "}
                        {provider.has_client_secret
                          ? t("Configured")
                          : t("Not configured")}
                      </FormLabel>
                      <FormControl>
                        <Input
                          type="password"
                          placeholder={t("Leave blank to keep current")}
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">{t("Only fill this in when rotating the Telegram bot token.")}</p>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </>
            )}

            {isApiKey && (
              <>
                <Separator className="my-2" />
                <h3 className="text-13 font-semibold">{t("API Key Configuration")}</h3>

                <FormField
                  control={form.control}
                  name="api_key_instructions"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{t("API Key Instructions")}</FormLabel>
                      <FormControl>
                        <textarea
                          className="flex min-h-[80px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-12 placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-input-focus disabled:cursor-not-allowed disabled:opacity-50"
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

            {(isOAuth || isDeviceCode) && (
              <div className="space-y-2 rounded-md border border-border p-3 text-xs">
                <p>{t("Token endpoint authentication:")}{" "}
                  {provider.token_endpoint_auth_method}
                </p>
                {isDeviceCode && (
                  <p>{t("Device code format: ")}{provider.device_code_format}</p>
                )}
                {provider.device_verification_url && (
                  <p className="break-all">{t("Device verification URL: ")}{provider.device_verification_url}
                  </p>
                )}
                {provider.extra_auth_params && (
                  <div>{t("Additional authorization parameters:")}<pre className="whitespace-pre-wrap">
                      {JSON.stringify(provider.extra_auth_params, null, 2)}
                    </pre>
                  </div>
                )}
                {provider.revocation && (
                  <p className="break-all">{t("Revocation: ")}{provider.revocation.style},{" "}
                    {provider.revocation.url}{t("; authentication:")}{" "}
                    {provider.revocation.auth}{t("; revokes grant:")}{" "}
                    {provider.revocation.revokes_grant ? t("Yes") : t("No")}
                  </p>
                )}
              </div>
            )}
            <div className="flex items-center justify-end gap-3 pt-4">
              <Button
                variant="primary"
                type="submit"
                isLoading={updateMutation.isPending}
                disabled={stale || !form.formState.isDirty}
              >{t("Save Changes")}</Button>
              <Button
                type="button"
                variant="outline"
                onClick={() =>
                  void navigate({
                    to: "/providers/$providerId",
                    params: { providerId },
                  })
                }
              >{t("Cancel")}</Button>
            </div>
          </form>
        </Form>
      </div>
    </div>
  );
}
