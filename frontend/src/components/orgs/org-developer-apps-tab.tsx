import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { Label, FieldHelp } from "@/components/ui/label";
import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { Code, Plus, Shield, ShieldCheck } from "lucide-react";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  useCreateDeveloperApp,
  useDeveloperApps,
} from "@/hooks/use-developer-apps";
import { parseRedirectUris } from "@/lib/oauth";
import { ApiError } from "@/lib/api-client";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { ClientSecretDialog } from "@/components/shared/client-secret-dialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { OrgReadOnlyRow } from "@/components/orgs/org-readonly-row";

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

interface OrgDeveloperAppsTabProps {
  readonly orgId: string;
  readonly orgName: string;
}

function StatCard({
  title,
  value,
  description,
  icon: Icon,
}: {
  readonly title: string;
  readonly value: string | number;
  readonly description: string;
  readonly icon: React.ComponentType<{ className?: string }>;
}) {
  useUiTranslation();
  return (
    <Card>
      <CardContent className="flex items-start justify-between p-4">
        <div className="space-y-1">
          <p className="text-14 text-muted-foreground">{title}</p>
          <p className="type-metric text-foreground">{value}</p>
          <p className="text-13 text-text-tertiary">{description}</p>
        </div>
        <Icon className="h-5 w-5 text-primary-text" />
      </CardContent>
    </Card>
  );
}

export function OrgDeveloperAppsTab({
  orgId,
  orgName,
}: OrgDeveloperAppsTabProps) {
  useUiTranslation();
  const navigate = useNavigate();
  const { data, isLoading, error, refetch } = useDeveloperApps(orgId);
  const createMutation = useCreateDeveloperApp();
  const [createOpen, setCreateOpen] = useState(false);
  const [showInactive, setShowInactive] = useState(false);
  const [name, setName] = useState("");
  const [redirectUrisText, setRedirectUrisText] = useState("");
  const [clientType, setClientType] = useState<"public" | "confidential">(
    "public",
  );
  const [selectedScopes, setSelectedScopes] = useState<readonly string[]>([
    "openid",
    "profile",
    "email",
  ]);
  const [secretOpen, setSecretOpen] = useState(false);
  const [createdClientId, setCreatedClientId] = useState("");
  const [createdClientSecret, setCreatedClientSecret] = useState("");

  const apps = data?.clients ?? [];
  const visibleApps = showInactive ? apps : apps.filter((app) => app.is_active);
  const activeCount = apps.filter((app) => app.is_active).length;
  const confidentialCount = apps.filter(
    (app) => app.client_type === "confidential",
  ).length;

  function resetCreateForm() {
    setName("");
    setRedirectUrisText("");
    setClientType("public");
    setSelectedScopes(["openid", "profile", "email"]);
  }

  async function handleCreate() {
    if (!name.trim()) {
      toast.error(translateErrorMessage("Application name is required"));
      return;
    }

    const parsedUris = parseRedirectUris(redirectUrisText);
    if (parsedUris.error) {
      toast.error(translateErrorMessage(parsedUris.error));
      return;
    }

    try {
      const created = await createMutation.mutateAsync({
        name: name.trim(),
        redirect_uris: parsedUris.uris,
        client_type: clientType,
        allowed_scopes: selectedScopes,
        target_org_id: orgId,
      });

      toast.success(translateUi("Developer app created"));
      if (created.client_secret) {
        setCreatedClientId(created.id);
        setCreatedClientSecret(created.client_secret);
        setSecretOpen(true);
      }
      setCreateOpen(false);
      resetCreateForm();
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError ? error.message : "Failed to create app"),
      );
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-end gap-2">
        <Button
          type="button"
          variant="outline"
          onClick={() => setShowInactive((current) => !current)}
        >
          {showInactive ? translateUi("Hide inactive") : translateUi("Show inactive")}
        </Button>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger asChild>
            <Button>
              <ButtonIcon><Plus className="h-4 w-4" /></ButtonIcon>{translateUi("New Application")}</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{translateUi("Create Developer App")}</DialogTitle>
              <DialogDescription>{translateUi("Register an OAuth application owned by ")}{orgName}.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-4">
              <OrgReadOnlyRow orgName={orgName} />
              <div className="space-y-2">
                <Label htmlFor="org-app-name">{translateUi("Application Name")}</Label>
                <Input
                  id="org-app-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder={translateUi("My SaaS App")}
                />
              </div>
              <div className="space-y-2">
                <Label

                  htmlFor="org-redirect-uris"
                >{translateUi("Redirect URIs (one per line)")}</Label>
                <textarea
                  id="org-redirect-uris"
                  value={redirectUrisText}
                  onChange={(event) => setRedirectUrisText(event.target.value)}
                  placeholder={
                    "https://app.example.com/oauth/callback\nmyapp://oauth/callback"
                  }
                  className="flex min-h-[120px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-14 placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-input-focus"
                />
              </div>
              <div className="space-y-2">
                <Label>{translateUi("Client Type")}</Label>
                <Select
                  value={clientType}
                  onValueChange={(value: "public" | "confidential") =>
                    setClientType(value)
                  }
                >
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="public">{translateUi("Public (PKCE)")}</SelectItem>
                    <SelectItem value="confidential">{translateUi("Confidential")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-3">
                <Label>{translateUi("Allowed Scopes")}</Label>
                <p className="text-13 text-muted-foreground">{translateUi("OIDC scopes this app can request. Determines what user data and NyxID capabilities are included in tokens.")}</p>
                <div className="space-y-2">
                  {OIDC_SCOPES.map((scope) => (
                    <div key={scope.id} className="flex items-start gap-2">
                      <Checkbox
                        id={`scope-org-create-${scope.id}`}
                        checked={selectedScopes.includes(scope.id)}
                        disabled={scope.required}
                        onCheckedChange={(checked) => {
                          setSelectedScopes(
                            checked
                              ? [...selectedScopes, scope.id]
                              : selectedScopes.filter((s) => s !== scope.id),
                          );
                        }}
                      />
                      <div className="grid gap-0.5">
                        <Label
                          htmlFor={`scope-org-create-${scope.id}`}
                          className="peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                        >
                          {scope.label}
                          {scope.required && (
                            <FieldHelp className="ml-1 text-muted-foreground">{translateUi("(required)")}</FieldHelp>
                          )}
                        </Label>
                        {"hint" in scope && scope.hint && (
                          <p className="text-13 text-muted-foreground">
                            {scope.hint}
                          </p>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setCreateOpen(false)}
              >{translateUi("Cancel")}</Button>
              <Button
                variant="primary"
                onClick={() => void handleCreate()}
                isLoading={createMutation.isPending}
              >{translateUi("Create App")}</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>

      <ClientSecretDialog
        open={secretOpen}
        onOpenChange={setSecretOpen}
        clientId={createdClientId}
        clientSecret={createdClientSecret}
      />

      <div className="grid gap-4 md:grid-cols-3">
        <StatCard
          title={translateUi("Total Apps")}
          value={apps.length}
          description={translateUi("OAuth clients owned by {{value1}}", { value1: orgName })}
          icon={Code}
        />
        <StatCard
          title={translateUi("Active Apps")}
          value={activeCount}
          description={translateUi("Ready for production traffic")}
          icon={ShieldCheck}
        />
        <StatCard
          title={translateUi("Confidential Apps")}
          value={confidentialCount}
          description={translateUi("Require client secret")}
          icon={Shield}
        />
      </div>

      <div className="grid gap-4 xl:grid-cols-2">
        {!isLoading && error && (
          <Card className="xl:col-span-2">
            <CardContent className="p-4">
              <ErrorBanner message={translateUi("Failed to load developer apps owned by {{value1}}.", { value1: orgName })} onRetry={refetch} />
            </CardContent>
          </Card>
        )}

        {isLoading &&
          Array.from({ length: 3 }).map((_, index) => (
            <Card key={`org-dev-app-skeleton-${String(index)}`}>
              <CardHeader>
                <Skeleton className="h-5 w-1/2" />
                <Skeleton className="h-4 w-3/4" />
              </CardHeader>
              <CardContent className="space-y-3">
                <Skeleton className="h-4 w-1/3" />
                <Skeleton className="h-4 w-2/3" />
                <Skeleton className="h-9 w-40" />
              </CardContent>
            </Card>
          ))}

        {!isLoading &&
          visibleApps.map((app) => (
            <Card
              key={app.id}
              className="cursor-pointer transition-colors duration-300 hover:border-hairline-strong"
              role="link"
              tabIndex={0}
              onClick={() =>
                void navigate({
                  to: "/orgs/$orgId/developer-apps/$clientId",
                  params: { orgId, clientId: app.id },
                })
              }
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") {
                  event.preventDefault();
                  void navigate({
                    to: "/orgs/$orgId/developer-apps/$clientId",
                    params: { orgId, clientId: app.id },
                  });
                }
              }}
            >
              <CardHeader className="space-y-2">
                <div className="flex items-center justify-between gap-3">
                  <CardTitle className="text-15">{app.client_name}</CardTitle>
                  <Badge variant={app.is_active ? "success" : "secondary"}>
                    {app.is_active ? translateUi("Active") : translateUi("Inactive")}
                  </Badge>
                </div>
                <CardDescription className="break-all">{translateUi("Client ID:")}{" "}
                  <span className="text-13 text-foreground">
                    {app.id}
                  </span>
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="secondary">{app.client_type}</Badge>
                  <Badge variant="secondary">
                    {app.redirect_uris.length}{translateUi(" redirect URIs")}</Badge>
                </div>
                <div className="flex flex-wrap gap-2">
                  {(app.allowed_scopes || "")
                    .split(/\s+/)
                    .filter(Boolean)
                    .map((scope) => (
                      <Badge
                        key={scope}
                        variant="secondary"
                        className="text-12"
                      >
                        {scope}
                      </Badge>
                    ))}
                </div>
              </CardContent>
            </Card>
          ))}

        {!isLoading && visibleApps.length === 0 && (
          <Card className="xl:col-span-2">
            <CardContent className="p-4 text-center">
              <p className="text-14 text-muted-foreground">
                {apps.length === 0
                  ? translateUi("No developer apps owned by {{value1}}.", { value1: orgName })
                  : translateUi("No active developer apps owned by {{value1}}. Enable \"Show inactive\" to view deactivated apps.", { value1: orgName })}
              </p>
            </CardContent>
          </Card>
        )}
      </div>
    </div>
  );
}
