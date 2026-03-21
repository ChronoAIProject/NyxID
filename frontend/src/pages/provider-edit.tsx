import { useEffect } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useForm, useWatch } from "react-hook-form";
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
import { AlertCircle } from "lucide-react";
import { toast } from "sonner";

const PROVIDER_TYPE_LABELS: Readonly<Record<string, string>> = {
  oauth2: "OAuth 2.0",
  api_key: "API Key",
  device_code: "Device Code",
  telegram_widget: "Telegram Widget",
};

function splitScopes(raw: string | undefined): readonly string[] | undefined {
  if (!raw || raw.trim() === "") return undefined;
  return raw
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

function stripEmptyStrings<T extends Record<string, unknown>>(
  obj: T,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(obj).filter(
      ([, v]) => v !== "" && v !== undefined,
    ),
  );
}

export function ProviderEditPage() {
  const { providerId } = useParams({ strict: false }) as {
    providerId: string;
  };
  const navigate = useNavigate();
  const { data: provider, isLoading, error } = useProvider(providerId);
  const updateMutation = useUpdateProvider(providerId);

  const form = useForm<UpdateProviderFormData>({
    resolver: zodResolver(updateProviderSchema),
    defaultValues: {
      name: "",
      slug: "",
      description: "",
      provider_type: "oauth2",
      credential_mode: "admin" as const,
      authorization_url: "",
      token_url: "",
      revocation_url: "",
      default_scopes: "",
      is_active: true,
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

  useEffect(() => {
    if (provider) {
      form.reset({
        name: provider.name,
        slug: provider.slug,
        description: provider.description ?? "",
        provider_type: provider.provider_type,
        credential_mode: provider.credential_mode ?? "admin",
        authorization_url: "",
        token_url: "",
        revocation_url: "",
        default_scopes:
          provider.default_scopes?.join(", ") ?? "",
        is_active: provider.is_active,
        client_id: "",
        client_secret: "",
        client_id_param_name: provider.client_id_param_name ?? "",
        supports_pkce: provider.supports_pkce,
        device_code_url: "",
        device_token_url: "",
        hosted_callback_url: provider.hosted_callback_url ?? "",
        api_key_instructions: provider.api_key_instructions ?? "",
        api_key_url: provider.api_key_url ?? "",
        icon_url: provider.icon_url ?? "",
        documentation_url: provider.documentation_url ?? "",
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider]);

  const watchedProviderType = useWatch({
    control: form.control,
    name: "provider_type",
  });

  async function onSubmit(data: UpdateProviderFormData) {
    if (!provider) return;
    try {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { slug: _, provider_type: _providerType, ...updateFields } = data;
      const cleaned = stripEmptyStrings({
        ...updateFields,
        default_scopes: splitScopes(data.default_scopes),
        supports_pkce:
          data.provider_type === "oauth2" ? data.supports_pkce : undefined,
      });
      await updateMutation.mutateAsync(
        cleaned as Parameters<typeof updateMutation.mutateAsync>[0],
      );
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

  if (error || !provider) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 font-display text-lg font-semibold">Provider not found</h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The provider you are trying to edit does not exist or has been deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/providers/manage" })}
        >
          Back to Providers
        </Button>
      </div>
    );
  }

  const isOAuth = watchedProviderType === "oauth2";
  const isDeviceCode = watchedProviderType === "device_code";
  const isApiKey = watchedProviderType === "api_key";
  const isTelegram = watchedProviderType === "telegram_widget";

  return (
    <div className="space-y-8">
      <PageHeader
        breadcrumbs={[
          { label: "Manage Providers", to: "/providers/manage" },
          {
            label: provider.name,
            to: `/providers/${providerId}`,
          },
          { label: "Edit" },
        ]}
        title={`Edit ${provider.name}`}
      />

      <div className="max-w-2xl">
        <Form {...form}>
          <form
            onSubmit={form.handleSubmit(onSubmit)}
            className="space-y-4"
          >
            {form.formState.errors.root && (
              <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
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
              <p className="text-sm font-medium mb-1">Slug</p>
              <Badge variant="secondary">{provider.slug}</Badge>
              <p className="text-xs text-muted-foreground mt-1">
                Slug cannot be changed after creation.
              </p>
            </div>

            <div>
              <p className="text-sm font-medium mb-1">Provider Type</p>
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
                      className="flex min-h-[80px] w-full rounded-[10px] border border-input bg-transparent px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
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
                      value={field.value ?? "admin"}
                      onValueChange={field.onChange}
                    >
                      <FormControl>
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        <SelectItem value="admin">
                          Admin Only
                        </SelectItem>
                        <SelectItem value="user">
                          User Provided
                        </SelectItem>
                        <SelectItem value="both">
                          Admin or User
                        </SelectItem>
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
                    <Input
                      placeholder="https://docs.example.com"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            {isOAuth && (
              <>
                <Separator className="my-2" />
                <h3 className="text-sm font-semibold">
                  OAuth 2.0 Configuration
                </h3>
                <p className="text-xs text-muted-foreground">
                  Leave URL fields blank to keep current values.
                </p>

                <FormField
                  control={form.control}
                  name="authorization_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Authorization URL</FormLabel>
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
                  name="token_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Token URL</FormLabel>
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
                  name="revocation_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Revocation URL</FormLabel>
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
                      <FormLabel>Client ID</FormLabel>
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
                      <FormLabel>Client Secret</FormLabel>
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
                <h3 className="text-sm font-semibold">
                  Device Code Configuration (RFC 8628)
                </h3>
                <p className="text-xs text-muted-foreground">
                  Leave URL fields blank to keep current values.
                </p>

                <FormField
                  control={form.control}
                  name="device_code_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Device Code URL</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="Leave blank to keep current"
                          {...field}
                        />
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
                        <Input
                          placeholder="Leave blank to keep current"
                          {...field}
                        />
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
                  name="token_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Token URL (fallback)</FormLabel>
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
                      <FormLabel>Client ID</FormLabel>
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
                <h3 className="text-sm font-semibold">
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
                        <Input
                          placeholder="NyxIdBot"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Enter the BotFather username without the leading
                        <span className="font-mono"> @</span>.
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
                      <FormLabel>Bot Token</FormLabel>
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
                <h3 className="text-sm font-semibold">
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
                          className="flex min-h-[80px] w-full rounded-[10px] border border-input bg-transparent px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
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

            <div className="flex items-center gap-3 pt-4">
              <Button type="submit" isLoading={updateMutation.isPending}>
                Save changes
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
