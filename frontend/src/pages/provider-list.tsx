import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm, useWatch } from "react-hook-form";
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
  DialogTrigger,
} from "@/components/ui/dialog";
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
import { Switch } from "@/components/ui/switch";
import { Separator } from "@/components/ui/separator";
import { Plus, Plug, Shield } from "lucide-react";
import { toast } from "sonner";

const PROVIDER_TYPE_LABELS: Readonly<Record<string, string>> = {
  oauth2: "OAuth 2.0",
  api_key: "API Key",
  device_code: "Device Code",
  telegram_widget: "Telegram Widget",
};

export function ProviderListPage() {
  const { data: providers, isLoading } = useProviders();
  const createMutation = useCreateProvider();
  const navigate = useNavigate();
  const [createOpen, setCreateOpen] = useState(false);

  const form = useForm<CreateProviderFormData>({
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
      await createMutation.mutateAsync(
        buildCreateProviderPayload(data) as Parameters<
          typeof createMutation.mutateAsync
        >[0],
      );
      toast.success("Provider created successfully");
      setCreateOpen(false);
      form.reset();
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to create provider");
      }
    }
  }

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="font-display text-3xl font-normal tracking-tight md:text-5xl">
            Manage Providers
          </h2>
          <p className="text-sm text-muted-foreground">
            Create and manage OAuth, Telegram, device code, and API key
            providers.
          </p>
        </div>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger asChild>
            <Button className="w-fit">
              <Plus className="mr-2 h-4 w-4" />
              Add Provider
            </Button>
          </DialogTrigger>
          <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
            <DialogHeader>
              <DialogTitle>Add Provider</DialogTitle>
              <DialogDescription>
                Register a new external provider for user connections.
              </DialogDescription>
            </DialogHeader>

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
                        <Input placeholder="GitHub" {...field} />
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
                      <FormLabel>Slug</FormLabel>
                      <FormControl>
                        <Input placeholder="github" {...field} />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Lowercase letters, digits, and hyphens only.
                      </p>
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
                          className="flex min-h-[60px] w-full rounded-[10px] border border-input bg-transparent px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
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
                  name="provider_type"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Provider Type</FormLabel>
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

                          if (resets.credential_mode !== undefined) {
                            form.setValue(
                              "credential_mode",
                              resets.credential_mode,
                              {
                                shouldDirty: false,
                                shouldValidate: false,
                              },
                            );
                          }

                          if (resets.client_id_param_name !== undefined) {
                            form.setValue(
                              "client_id_param_name",
                              resets.client_id_param_name,
                              {
                                shouldDirty: false,
                                shouldValidate: false,
                              },
                            );
                          }

                          if (
                            resets.credential_mode !== undefined ||
                            resets.client_id_param_name !== undefined
                          ) {
                            form.clearErrors([
                              "credential_mode",
                              "client_id_param_name",
                            ]);
                          }
                        }}
                      >
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue placeholder="Select provider type" />
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
                            <SelectItem value="admin">Admin Only</SelectItem>
                            <SelectItem value="user">User Provided</SelectItem>
                            <SelectItem value="both">Admin or User</SelectItem>
                          </SelectContent>
                        </Select>
                        <p className="text-xs text-muted-foreground">
                          Choose whether users rely on admin-managed OAuth apps,
                          bring their own, or can use either.
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

                {watchedProviderType === "oauth2" && (
                  <>
                    <Separator />
                    <h4 className="text-sm font-semibold">
                      OAuth 2.0 Configuration
                    </h4>

                    <FormField
                      control={form.control}
                      name="authorization_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>Authorization URL</FormLabel>
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
                          <FormLabel>Token URL</FormLabel>
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
                          <FormLabel>Revocation URL</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://provider.com/oauth/revoke"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            Optional. Used to revoke tokens on disconnect.
                          </p>
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
                            <Input placeholder="OAuth client ID" {...field} />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            {watchedCredentialMode === "admin"
                              ? "Required in admin mode."
                              : "Optional. Leave blank to require users to supply their own OAuth app credentials."}
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
                          <FormLabel>Client Secret</FormLabel>
                          <FormControl>
                            <Input
                              type="password"
                              placeholder="OAuth client secret"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            {watchedCredentialMode === "admin"
                              ? "Required for OAuth 2.0 admin credentials."
                              : "Optional. Only set this when configuring an admin fallback OAuth app."}
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

                {watchedProviderType === "device_code" && (
                  <>
                    <Separator />
                    <h4 className="text-sm font-semibold">
                      Device Code Configuration (RFC 8628)
                    </h4>

                    <FormField
                      control={form.control}
                      name="device_code_url"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>Device Code URL</FormLabel>
                          <FormControl>
                            <Input
                              placeholder="https://auth.openai.com/deviceauth/usercode"
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
                              placeholder="https://auth.openai.com/deviceauth/token"
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
                          <FormLabel>Token URL (fallback)</FormLabel>
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
                              placeholder="app_EMoamEEZ73f0CkXaXp7hrann"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            {watchedCredentialMode === "admin"
                              ? "Required in admin mode."
                              : "Optional. Needed only when configuring an admin fallback client."}
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
                          <FormLabel>Client Secret (optional)</FormLabel>
                          <FormControl>
                            <Input
                              type="password"
                              placeholder="OAuth client secret (if required)"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            Optional for public OAuth clients.
                          </p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </>
                )}

                {watchedProviderType === "telegram_widget" && (
                  <>
                    <Separator />
                    <h4 className="text-sm font-semibold">
                      Telegram Widget Configuration
                    </h4>

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
                              placeholder="123456:ABC-DEF1234567890"
                              {...field}
                            />
                          </FormControl>
                          <p className="text-xs text-muted-foreground">
                            BotFather token used to verify Telegram Login Widget
                            callbacks.
                          </p>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </>
                )}

                {watchedProviderType === "api_key" && (
                  <>
                    <Separator />
                    <h4 className="text-sm font-semibold">
                      API Key Configuration
                    </h4>

                    <FormField
                      control={form.control}
                      name="api_key_instructions"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>API Key Instructions</FormLabel>
                          <FormControl>
                            <textarea
                              className="flex min-h-[60px] w-full rounded-[10px] border border-input bg-transparent px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
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

                <DialogFooter>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setCreateOpen(false)}
                  >
                    Cancel
                  </Button>
                  <Button type="submit" isLoading={createMutation.isPending}>
                    Create provider
                  </Button>
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
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <Shield className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No providers configured yet. Add a provider to get started.
          </p>
        </div>
      ) : (
        <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
          {providers.map((provider) => (
            <Card
              key={provider.id}
              className="cursor-pointer transition-colors hover:border-border/80"
              onClick={() =>
                void navigate({
                  to: "/providers/$providerId",
                  params: { providerId: provider.id },
                })
              }
            >
              <CardHeader className="flex flex-row items-start justify-between space-y-0 pb-3">
                <div className="flex items-center gap-3">
                  <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-primary/10">
                    <Plug className="h-5 w-5 text-primary" />
                  </div>
                  <div>
                    <CardTitle className="text-base">{provider.name}</CardTitle>
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
                        {provider.is_active ? "Active" : "Inactive"}
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
