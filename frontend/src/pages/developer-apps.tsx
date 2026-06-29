import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import {
  useCreateDeveloperApp,
  useDeveloperApps,
} from "@/hooks/use-developer-apps";
import { parseRedirectUris } from "@/lib/oauth";
import { ErrorBanner } from "@/components/shared/error-banner";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
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
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Checkbox } from "@/components/ui/checkbox";
import { ClientSecretDialog } from "@/components/shared/client-secret-dialog";
import { ApiError } from "@/lib/api-client";
import { Code, ExternalLink, Shield, ShieldCheck } from "lucide-react";
import { WebsiteLayoutIcon } from "@/components/icons/empty-state";

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
          <p className="text-[11px] text-text-tertiary">{description}</p>
        </div>
        <Icon className="h-5 w-5 text-primary" />
      </CardContent>
    </Card>
  );
}

export function DeveloperAppsPage() {
  const navigate = useNavigate();
  const { data, isLoading, error, refetch } = useDeveloperApps();
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
      });

      toast.success("Developer app created");
      if (created.client_secret) {
        setCreatedClientId(created.id);
        setCreatedClientSecret(created.client_secret);
        setSecretOpen(true);
      }
      setCreateOpen(false);
      setName("");
      setRedirectUrisText("");
      setClientType("public");
      setSelectedScopes(["openid", "profile", "email"]);
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to create app");
      }
    }
  }

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h2 className="text-[28px] font-bold leading-none tracking-tight" style={{ letterSpacing: "-0.03em" }}>
            Developer Apps
          </h2>
          <p className="text-[12px] text-muted-foreground">
            Register and manage OAuth applications for your products.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-2">
            <Switch
              id="show-inactive"
              checked={showInactive}
              onCheckedChange={setShowInactive}
            />
            <label htmlFor="show-inactive" className="text-[12px] text-muted-foreground cursor-pointer">
              Show inactive
            </label>
          </div>
          <AddCtaButton label="New Application" onClick={() => setCreateOpen(true)} />
          <Dialog open={createOpen} onOpenChange={setCreateOpen}>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>Create Developer App</DialogTitle>
                <DialogDescription>
                  Register a new OAuth application for your product.
                </DialogDescription>
              </DialogHeader>
              <div className="space-y-4">
                <div className="space-y-2">
                  <label className="text-[12px] font-medium" htmlFor="app-name">
                    Application Name
                  </label>
                  <Input
                    id="app-name"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    placeholder="My SaaS App"
                  />
                </div>
                <div className="space-y-2">
                  <label
                    className="text-[12px] font-medium"
                    htmlFor="redirect-uris"
                  >
                    Redirect URIs (one per line)
                  </label>
                  <textarea
                    id="redirect-uris"
                    value={redirectUrisText}
                    onChange={(event) =>
                      setRedirectUrisText(event.target.value)
                    }
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
                          id={`scope-create-${scope.id}`}
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
                            htmlFor={`scope-create-${scope.id}`}
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
      </div>

      <div className="grid gap-4 md:grid-cols-3">
        <StatCard
          title="Total Apps"
          value={apps.length}
          description="All registered applications"
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
          <div className="xl:col-span-2">
            <ErrorBanner message="Failed to load developer apps. Please refresh and try again." onRetry={refetch} />
          </div>
        )}

        {isLoading &&
          Array.from({ length: 3 }).map((_, index) => (
            <Card key={`skeleton-${index}`}>
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
            <Card key={app.id}>
              <CardHeader className="space-y-2">
                <div className="flex items-center justify-between gap-3">
                  <CardTitle>{app.client_name}</CardTitle>
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
                <div className="flex justify-end">
                  <Button
                    variant="outline"
                    className="text-text-tertiary hover:text-muted-foreground"
                    onClick={() =>
                      void navigate({
                        to: "/developer/apps/$clientId",
                        params: { clientId: app.id },
                      })
                    }
                  >
                    <ButtonIcon><ExternalLink className="h-3 w-3" /></ButtonIcon>
                    View Details
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}

        {!isLoading && visibleApps.length === 0 && (
          <div className="xl:col-span-2 flex flex-col items-center justify-center gap-1 py-12 text-center">
            <WebsiteLayoutIcon className="h-64 w-64 text-muted-foreground" />
            <div className="space-y-1">
              <p className="text-[12px] font-medium text-muted-foreground">
                {apps.length === 0 ? "No Developer Apps" : "No Active Apps"}
              </p>
              <p className="text-xs text-muted-foreground">
                {apps.length === 0
                  ? "Create your first application."
                  : "Enable 'Show inactive' to view deactivated apps."}
              </p>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
