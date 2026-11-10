import { getFormattingLocale } from "@/lib/formatting-locale";
import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  useDeleteDeveloperApp,
  useDeveloperApp,
  useRotateDeveloperAppSecret,
  useUpdateDeveloperApp,
} from "@/hooks/use-developer-apps";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { Checkbox } from "@/components/ui/checkbox";
import { ClientSecretDialog } from "@/components/shared/client-secret-dialog";
import { PageHeader } from "@/components/shared/page-header";
import { parseRedirectUris } from "@/lib/oauth";
import { ApiError } from "@/lib/api-client";
import { toast } from "sonner";
import { useCatalog } from "@/hooks/use-keys";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { ConnectionWebhookSection } from "./connection-webhook-section";

const OIDC_SCOPES = [
  { id: "openid", label: "openid", required: true },
  { id: "profile", label: "profile", required: false },
  { id: "email", label: "email", required: false },
  {
    id: "offline_access",
    label: "offline_access",
    required: false,
    hint: "Allows refresh tokens for durable browser sessions",
  },
  {
    id: "proxy",
    label: "proxy",
    required: false,
    hint: "Allows access to NyxID proxy, LLM gateway, and MCP tools",
  },
  {
    id: "roles",
    label: "roles",
    required: false,
    hint: "Includes user roles and permissions in tokens",
  },
  {
    id: "groups",
    label: "groups",
    required: false,
    hint: "Includes user group memberships in tokens",
  },
] as const;

interface DeveloperAppDetailProps {
  readonly clientId: string;
  readonly backTo: { readonly to: string; readonly label: string; readonly search?: { tab: string } };
}

export function DeveloperAppDetail({
  clientId,
  backTo,
}: DeveloperAppDetailProps) {
  const { t } = useUiTranslation();
  const navigate = useNavigate();
  const { data: app, isLoading } = useDeveloperApp(clientId);
  useBreadcrumbLabel(app?.client_name);
  const updateMutation = useUpdateDeveloperApp();
  const deleteMutation = useDeleteDeveloperApp();
  const rotateMutation = useRotateDeveloperAppSecret();

  const { data: catalog } = useCatalog({ includeAll: true });

  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [name, setName] = useState("");
  const [redirectUrisText, setRedirectUrisText] = useState("");
  const [editScopes, setEditScopes] = useState<readonly string[]>([]);
  const [editDefaultSlugs, setEditDefaultSlugs] = useState<readonly string[]>(
    [],
  );

  const [secretOpen, setSecretOpen] = useState(false);
  const [rotatedSecret, setRotatedSecret] = useState("");

  function openEditDialog() {
    if (!app) return;
    setName(app.client_name);
    setRedirectUrisText(app.redirect_uris.join("\n"));
    setEditScopes(app.allowed_scopes.split(/\s+/).filter(Boolean));
    setEditDefaultSlugs(app.default_service_catalog_slugs ?? []);
    setEditOpen(true);
  }

  function catalogServiceName(slug: string): string {
    return (catalog ?? []).find((entry) => entry.slug === slug)?.name ?? slug;
  }

  async function handleSave() {
    if (!app) return;
    const parsedUris = parseRedirectUris(redirectUrisText);

    if (!name.trim()) {
      toast.error(translateUi("Name is required"));
      return;
    }

    if (parsedUris.error) {
      toast.error(translateErrorMessage(parsedUris.error));
      return;
    }

    try {
      await updateMutation.mutateAsync({
        clientId: app.id,
        data: {
          name: name.trim(),
          redirect_uris: parsedUris.uris,
          allowed_scopes: editScopes,
          default_service_catalog_slugs: editDefaultSlugs,
        },
      });
      toast.success(translateUi("Application updated"));
      setEditOpen(false);
    } catch (error) {
      toast.error(translateErrorMessage(error instanceof ApiError ? error.message : "Update failed"));
    }
  }

  async function handleRotateSecret() {
    if (!app) return;
    try {
      const result = await rotateMutation.mutateAsync(app.id);
      setRotatedSecret(result.client_secret);
      setRotateOpen(false);
      setSecretOpen(true);
      toast.success(translateUi("Client secret rotated"));
    } catch (error) {
      toast.error(translateErrorMessage(error instanceof ApiError ? error.message : "Rotate failed"));
    }
  }

  async function handleDelete() {
    if (!app) return;
    try {
      await deleteMutation.mutateAsync(app.id);
      toast.success(translateUi("Application deactivated"));
      setDeleteOpen(false);
      void navigate({ to: backTo.to, search: () => backTo.search ?? {} });
    } catch (error) {
      toast.error(translateErrorMessage(error instanceof ApiError ? error.message : "Delete failed"));
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-56" />
        <Skeleton className="h-48 w-full" />
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  if (!app) {
    return (
      <div className="space-y-4">
        <h2 className="text-3xl md:text-5xl font-normal tracking-tight">{t("Developer App")}</h2>
        <p className="text-muted-foreground">{t("App not found or you do not have access.")}</p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: backTo.to, search: () => backTo.search ?? {} })}
        >{t("Back to ")}{backTo.label}
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={app.client_name}
        description={t("Client details, redirect URIs, and OAuth metadata.")}
        actions={
          <>
            <Button variant="outline" onClick={openEditDialog}>{t("Edit")}</Button>
            {app.client_type === "confidential" && (
              <Button
                variant="outline"
                onClick={() => setRotateOpen(true)}
              >{t("Rotate Secret")}</Button>
            )}
            <Button variant="destructive" onClick={() => setDeleteOpen(true)}>{t("Deactivate")}</Button>
          </>
        }
      />

      <Card>
        <CardHeader>
          <CardTitle>{t("Credentials")}</CardTitle>
          <CardDescription>{t("These values identify your OAuth client.")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-2">
            <Badge variant="secondary">{app.client_type}</Badge>
            <Badge variant={app.is_active ? "success" : "secondary"}>
              {app.is_active ? t("Active") : t("Inactive")}
            </Badge>
          </div>
          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">{t("Client ID")}</p>
            <p className="break-all font-mono text-12 text-foreground">
              {app.id}
            </p>
          </div>
          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">{t("Created At")}</p>
            <p className="text-12 text-foreground">
              {new Date(app.created_at).toLocaleString(getFormattingLocale())}
            </p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("Redirect URIs")}</CardTitle>
          <CardDescription>{t("OAuth callback destinations configured for this client.")}</CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {app.redirect_uris.map((uri) => (
            <div
              key={uri}
              className="break-all rounded-lg border border-border bg-muted px-3 py-2 font-mono text-xs"
            >
              {uri}
            </div>
          ))}
          {app.redirect_uris.length === 0 && (
            <p className="text-12 text-muted-foreground">{t("No redirect URIs configured.")}</p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("Allowed Scopes")}</CardTitle>
          <CardDescription>{t("OIDC scopes this client can request. Determines what user data and NyxID capabilities are included in tokens.")}</CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex flex-wrap gap-2">
            {app.allowed_scopes
              .split(/\s+/)
              .filter(Boolean)
              .map((scope) => (
                <Badge key={scope} variant="secondary">
                  {scope}
                </Badge>
              ))}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{t("Default Services")}</CardTitle>
          <CardDescription>{t("Services this app requests by default. The consent screen pre-selects the user's matching services; users can always adjust before approving.")}</CardDescription>
        </CardHeader>
        <CardContent>
          {(app.default_service_catalog_slugs ?? []).length > 0 ? (
            <div className="flex flex-wrap gap-2">
              {(app.default_service_catalog_slugs ?? []).map((slug) => (
                <Badge key={slug} variant="secondary">
                  {catalogServiceName(slug)}
                </Badge>
              ))}
            </div>
          ) : (
            <p className="text-12 text-muted-foreground">{t("No default services declared. Users see a sign-in-only consent unless they customize.")}</p>
          )}
        </CardContent>
      </Card>

      <ConnectionWebhookSection
        clientId={app.id}
        webhookUrl={app.connection_webhook_url}
        enabled={app.connection_webhook_enabled}
      />

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Edit Developer App")}</DialogTitle>
            <DialogDescription>{t("Update application name and redirect URIs.")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <label className="text-12 font-medium" htmlFor="edit-app-name">{t("Application Name")}</label>
              <Input
                id="edit-app-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
            </div>
            <div className="space-y-2">
              <label
                className="text-12 font-medium"
                htmlFor="edit-redirect-uris"
              >{t("Redirect URIs (one per line)")}</label>
              <textarea
                id="edit-redirect-uris"
                value={redirectUrisText}
                onChange={(event) => setRedirectUrisText(event.target.value)}
                className="flex min-h-[120px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-12 placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-input-focus"
              />
            </div>
            <div className="space-y-3">
              <label className="text-12 font-medium">{t("Allowed Scopes")}</label>
              <div className="space-y-2">
                {OIDC_SCOPES.map((scope) => (
                  <div key={scope.id} className="flex items-start gap-2">
                    <Checkbox
                      id={`scope-edit-${scope.id}`}
                      checked={editScopes.includes(scope.id)}
                      disabled={scope.required}
                      onCheckedChange={(checked) => {
                        setEditScopes(
                          checked
                            ? [...editScopes, scope.id]
                            : editScopes.filter((s) => s !== scope.id),
                        );
                      }}
                    />
                    <div className="grid gap-0.5 leading-none">
                      <label
                        htmlFor={`scope-edit-${scope.id}`}
                        className="text-12 font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                      >
                        {scope.label}
                        {scope.required && (
                          <span className="ml-1 text-xs text-muted-foreground">{t("(required)")}</span>
                        )}
                      </label>
                      {"hint" in scope && scope.hint && (
                        <p className="text-xs text-muted-foreground">
                          {scope.hint}
                        </p>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </div>
            <div className="space-y-3">
              <div>
                <label className="text-12 font-medium">{t("Default Services")}</label>
                <p className="text-xs text-muted-foreground">{t("Catalog services this app requests at consent time. Users keep the final say.")}</p>
              </div>
              <div className="max-h-48 space-y-2 overflow-y-auto pr-1">
                {(catalog ?? []).map((entry) => (
                  <div key={entry.slug} className="flex items-start gap-2">
                    <Checkbox
                      id={`default-service-${entry.slug}`}
                      checked={editDefaultSlugs.includes(entry.slug)}
                      onCheckedChange={(checked) => {
                        setEditDefaultSlugs(
                          checked
                            ? [...editDefaultSlugs, entry.slug]
                            : editDefaultSlugs.filter(
                                (slug) => slug !== entry.slug,
                              ),
                        );
                      }}
                    />
                    <label
                      htmlFor={`default-service-${entry.slug}`}
                      className="cursor-pointer text-12 font-medium leading-none"
                    >
                      {entry.name}
                      <span className="ml-1 text-xs font-normal text-muted-foreground">
                        {entry.slug}
                      </span>
                    </label>
                  </div>
                ))}
                {(catalog ?? []).length === 0 && (
                  <p className="text-xs text-muted-foreground">{t("No catalog services available.")}</p>
                )}
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditOpen(false)}>{t("Cancel")}</Button>
            <Button
              variant="primary"
              onClick={() => void handleSave()}
              isLoading={updateMutation.isPending}
            >{t("Save")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ClientSecretDialog
        open={secretOpen}
        onOpenChange={setSecretOpen}
        clientSecret={rotatedSecret}
      />

      <Dialog open={deleteOpen} onOpenChange={setDeleteOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Deactivate Application")}</DialogTitle>
            <DialogDescription>{t("This will disable OAuth authorization for this app. Existing tokens may stop working depending on downstream policy.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>{t("Cancel")}</Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >{t("Deactivate")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={rotateOpen} onOpenChange={setRotateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Rotate client secret?")}</DialogTitle>
            <DialogDescription>{t("Your old client secret will stop working immediately. Every deployment using it will fail to authenticate until you update them with the new secret.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRotateOpen(false)}>{t("Cancel")}</Button>
            <Button
              variant="primary"
              onClick={() => void handleRotateSecret()}
              isLoading={rotateMutation.isPending}
            >{t("Rotate Secret")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
