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
  return (
    <Card>
      <CardContent className="flex items-start justify-between p-4">
        <div className="space-y-1">
          <p className="text-[12px] text-muted-foreground">{title}</p>
          <p className="text-2xl font-semibold text-foreground">{value}</p>
          <p className="text-xs text-text-tertiary">{description}</p>
        </div>
        <Icon className="h-5 w-5 text-primary" />
      </CardContent>
    </Card>
  );
}

export function OrgDeveloperAppsTab({
  orgId,
  orgName,
}: OrgDeveloperAppsTabProps) {
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
      toast.error("Application name is required");
      return;
    }

    const parsedUris = parseRedirectUris(redirectUrisText);
    if (parsedUris.error) {
      toast.error(parsedUris.error);
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

      toast.success("Developer app created");
      if (created.client_secret) {
        setCreatedClientId(created.id);
        setCreatedClientSecret(created.client_secret);
        setSecretOpen(true);
      }
      setCreateOpen(false);
      resetCreateForm();
    } catch (error) {
      toast.error(
        error instanceof ApiError ? error.message : "Failed to create app",
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
          {showInactive ? "Hide inactive" : "Show inactive"}
        </Button>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger asChild>
            <Button>
              <ButtonIcon><Plus className="h-4 w-4" /></ButtonIcon>
              New Application
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Developer App</DialogTitle>
              <DialogDescription>
                Register an OAuth application owned by {orgName}.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-4">
              <OrgReadOnlyRow orgName={orgName} />
              <div className="space-y-2">
                <label className="text-[12px] font-medium" htmlFor="org-app-name">
                  Application Name
                </label>
                <Input
                  id="org-app-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="My SaaS App"
                />
              </div>
              <div className="space-y-2">
                <label
                  className="text-[12px] font-medium"
                  htmlFor="org-redirect-uris"
                >
                  Redirect URIs (one per line)
                </label>
                <textarea
                  id="org-redirect-uris"
                  value={redirectUrisText}
                  onChange={(event) => setRedirectUrisText(event.target.value)}
                  placeholder={
                    "https://app.example.com/oauth/callback\nmyapp://oauth/callback"
                  }
                  className="flex min-h-[120px] w-full rounded-lg border border-input bg-transparent px-3 py-2 text-[12px] placeholder:text-muted-foreground focus-visible:outline-none"
                />
              </div>
              <div className="space-y-2">
                <label className="text-[12px] font-medium">Client Type</label>
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
                    <SelectItem value="public">Public (PKCE)</SelectItem>
                    <SelectItem value="confidential">Confidential</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-3">
                <label className="text-[12px] font-medium">Allowed Scopes</label>
                <p className="text-xs text-muted-foreground">
                  OIDC scopes this app can request. Determines what user data
                  and NyxID capabilities are included in tokens.
                </p>
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
                      <div className="grid gap-0.5 leading-none">
                        <label
                          htmlFor={`scope-org-create-${scope.id}`}
                          className="text-[12px] font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                        >
                          {scope.label}
                          {scope.required && (
                            <span className="ml-1 text-xs text-muted-foreground">
                              (required)
                            </span>
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
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setCreateOpen(false)}
              >
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={() => void handleCreate()}
                isLoading={createMutation.isPending}
              >
                Create App
              </Button>
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
          title="Total Apps"
          value={apps.length}
          description={`OAuth clients owned by ${orgName}`}
          icon={Code}
        />
        <StatCard
          title="Active Apps"
          value={activeCount}
          description="Ready for production traffic"
          icon={ShieldCheck}
        />
        <StatCard
          title="Confidential Apps"
          value={confidentialCount}
          description="Require client secret"
          icon={Shield}
        />
      </div>

      <div className="grid gap-4 xl:grid-cols-2">
        {!isLoading && error && (
          <Card className="xl:col-span-2">
            <CardContent className="p-4">
              <ErrorBanner message={`Failed to load developer apps owned by ${orgName}.`} onRetry={refetch} />
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
              className="cursor-pointer transition-colors duration-300 hover:border-white/[0.15]"
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
                  <CardTitle className="text-base">{app.client_name}</CardTitle>
                  <Badge variant={app.is_active ? "success" : "secondary"}>
                    {app.is_active ? "Active" : "Inactive"}
                  </Badge>
                </div>
                <CardDescription className="break-all">
                  Client ID:{" "}
                  <span className="text-xs text-foreground">
                    {app.id}
                  </span>
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge variant="secondary">{app.client_type}</Badge>
                  <Badge variant="secondary">
                    {app.redirect_uris.length} redirect URIs
                  </Badge>
                </div>
                <div className="flex flex-wrap gap-2">
                  {(app.allowed_scopes || "")
                    .split(/\s+/)
                    .filter(Boolean)
                    .map((scope) => (
                      <Badge
                        key={scope}
                        variant="secondary"
                        className="text-xs"
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
              <p className="text-[12px] text-muted-foreground">
                {apps.length === 0
                  ? `No developer apps owned by ${orgName}.`
                  : `No active developer apps owned by ${orgName}. Enable "Show inactive" to view deactivated apps.`}
              </p>
            </CardContent>
          </Card>
        )}
      </div>
    </div>
  );
}

