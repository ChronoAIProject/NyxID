import { ProviderServices } from "@/components/providers/provider-services";
import { useState } from "react";
import type { ProviderConfig } from "@/types/api";
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
  const { providerId } = useParams({ strict: false }) as { providerId: string };
  const { data: provider, isLoading, error, refetch } = useProvider(providerId);
  if (isLoading && !provider) return <Skeleton className="h-96 w-full" />;
  if (!provider)
    return (
      <ErrorBanner
        message={
          error instanceof ApiError ? error.message : "Unable to load provider"
        }
        onRetry={refetch}
      />
    );
  return <ProviderEditForm key={providerId} source={provider} />;
}

function ProviderEditForm({ source }: { readonly source: ProviderConfig }) {
  const [provider, setProvider] = useState(source);
  const providerId = provider.id;
  const isMounted = useEditorMounted();
  const navigate = useNavigate();
  const updateMutation = useUpdateProvider(providerId);

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
      toast.success("Provider updated");
      void navigate({
        to: "/providers/$providerId",
        params: { providerId },
      });
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error("Failed to update provider");
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
      <PageHeader title={`Edit ${provider.name}`} />

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
              <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
                {form.formState.errors.root.message}
              </div>
            )}

            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Name</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <div>
              <p className="text-[12px] font-medium mb-1">Slug</p>
              <Badge variant="secondary">{provider.slug}</Badge>
              <p className="text-xs text-muted-foreground mt-1">
                Slug cannot be changed after creation.
              </p>
            </div>

            <div>
              <p className="text-[12px] font-medium mb-1">Provider Type</p>
              <Badge variant="secondary">
                {PROVIDER_TYPE_LABELS[provider.provider_type] ??
                  provider.provider_type}
              </Badge>
              <p className="text-xs text-muted-foreground mt-1">
                Provider type cannot be changed after creation.
              </p>
            </div>

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
              name="is_active"
              render={({ field }) => (
                <FormItem className="flex items-center justify-between rounded-lg border p-3">
                  <div className="space-y-0.5">
                    <FormLabel>Active</FormLabel>
                    <p className="text-xs text-muted-foreground">
                      Inactive providers will not be available for user
                      connections.
                    </p>
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
                    <FormLabel>Credential Mode</FormLabel>
                    <Select
                      value={field.value || "admin"}
                      onValueChange={field.onChange}
                    >
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue placeholder="Admin Only" />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="admin">Admin Only</SelectItem>
                        <SelectItem value="user">User Provided</SelectItem>
                        <SelectItem value="both">Admin or User</SelectItem>
                      </SelectContent>
                    </Select>
                    <p className="text-xs text-muted-foreground">
                      Controls whether admin-configured or user-provided OAuth
                      credentials are used for connections.
                    </p>
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
                  <FormLabel>Icon URL</FormLabel>
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
                  <FormLabel>Documentation URL</FormLabel>
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
                <h3 className="text-[13px] font-semibold">
                  OAuth 2.0 Configuration
                </h3>
                <p className="text-xs text-muted-foreground">
                  Saved endpoint URLs are shown below.
                </p>

                <FormField
                  control={form.control}
                  name="authorization_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Authorization URL</FormLabel>
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
                      <FormLabel>Token URL</FormLabel>
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
                      <FormLabel>Revocation URL</FormLabel>
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
                      <FormLabel>Default Scopes</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="read, write, user:email"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Comma-separated list of scopes.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="client_id"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        Client ID —{" "}
                        {provider.has_client_id
                          ? "Configured"
                          : "Not configured"}
                      </FormLabel>
                      <FormControl>
                        <Input
                          placeholder="Leave blank to keep current"
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
                      <FormLabel>
                        Client Secret —{" "}
                        {provider.has_client_secret
                          ? "Configured"
                          : "Not configured"}
                      </FormLabel>
                      <FormControl>
                        <Input
                          type="password"
                          placeholder="Leave blank to keep current"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Only fill in if you want to change the client secret.
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
                        <FormLabel>Supports PKCE</FormLabel>
                        <p className="text-xs text-muted-foreground">
                          Enable Proof Key for Code Exchange.
                        </p>
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
                <h3 className="text-[13px] font-semibold">
                  Device Code Configuration (RFC 8628)
                </h3>
                <p className="text-xs text-muted-foreground">
                  Saved endpoint URLs are shown below.
                </p>

                <FormField
                  control={form.control}
                  name="device_code_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Device Code URL</FormLabel>
                      <FormControl>
                        <Input placeholder="https://…" {...field} />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Endpoint to request a device code (RFC 8628 step 1).
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="device_token_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Device Token URL</FormLabel>
                      <FormControl>
                        <Input placeholder="https://…" {...field} />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Endpoint to poll for token (RFC 8628 step 3).
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="authorization_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Authorization URL (fallback)</FormLabel>
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
                      <FormLabel>Token URL (fallback)</FormLabel>
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
                      <FormLabel>Default Scopes</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="openid, profile, offline_access"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Comma-separated list of scopes.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="client_id"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        Client ID —{" "}
                        {provider.has_client_id
                          ? "Configured"
                          : "Not configured"}
                      </FormLabel>
                      <FormControl>
                        <Input
                          placeholder="Leave blank to keep current"
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
                      <FormLabel>Client Secret (optional)</FormLabel>
                      <FormControl>
                        <Input
                          type="password"
                          placeholder="Leave blank to keep current"
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
                <h3 className="text-[13px] font-semibold">
                  Telegram Widget Configuration
                </h3>
                <p className="text-xs text-muted-foreground">
                  Leave the bot token blank to keep the current secret.
                </p>

                <FormField
                  control={form.control}
                  name="client_id_param_name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Bot Username</FormLabel>
                      <FormControl>
                        <Input placeholder="NyxIdBot" {...field} />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Enter the BotFather username. A leading
                        <span> @</span> is optional.
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
                      <FormLabel>
                        Bot Token —{" "}
                        {provider.has_client_secret
                          ? "Configured"
                          : "Not configured"}
                      </FormLabel>
                      <FormControl>
                        <Input
                          type="password"
                          placeholder="Leave blank to keep current"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Only fill this in when rotating the Telegram bot token.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </>
            )}

            {isApiKey && (
              <>
                <Separator className="my-2" />
                <h3 className="text-[13px] font-semibold">
                  API Key Configuration
                </h3>

                <FormField
                  control={form.control}
                  name="api_key_instructions"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>API Key Instructions</FormLabel>
                      <FormControl>
                        <textarea
                          className="flex min-h-[80px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-[12px] placeholder:text-muted-foreground focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-50"
                          placeholder="Instructions for users to obtain an API key"
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
                      <FormLabel>API Key URL</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="https://provider.com/api-keys"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Link where users can generate an API key.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </>
            )}

            {(isOAuth || isDeviceCode) && (
              <div className="space-y-2 rounded-md border border-border p-3 text-xs">
                <p>
                  Token endpoint authentication:{" "}
                  {provider.token_endpoint_auth_method}
                </p>
                {isDeviceCode && (
                  <p>Device code format: {provider.device_code_format}</p>
                )}
                {provider.device_verification_url && (
                  <p className="break-all">
                    Device verification URL: {provider.device_verification_url}
                  </p>
                )}
                {provider.extra_auth_params && (
                  <div>
                    Additional authorization parameters:
                    <pre className="whitespace-pre-wrap">
                      {JSON.stringify(provider.extra_auth_params, null, 2)}
                    </pre>
                  </div>
                )}
                {provider.revocation && (
                  <p className="break-all">
                    Revocation: {provider.revocation.style},{" "}
                    {provider.revocation.url}; authentication:{" "}
                    {provider.revocation.auth}; revokes grant:{" "}
                    {provider.revocation.revokes_grant ? "Yes" : "No"}
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
              >
                Save Changes
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() =>
                  void navigate({
                    to: "/providers/$providerId",
                    params: { providerId },
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
