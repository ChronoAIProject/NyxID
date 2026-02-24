import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { toast } from "sonner";
import { useCreateDeveloperApp, useDeveloperApps } from "@/hooks/use-developer-apps";
import { parseRedirectUris } from "@/lib/oauth";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import { ClientSecretDialog } from "@/components/shared/client-secret-dialog";
import { ApiError } from "@/lib/api-client";
import { Code, Plus, Shield, ShieldCheck } from "lucide-react";

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
      <CardContent className="flex items-start justify-between p-6">
        <div className="space-y-1">
          <p className="text-sm text-muted-foreground">{title}</p>
          <p className="text-2xl font-semibold text-foreground">{value}</p>
          <p className="text-xs text-text-tertiary">{description}</p>
        </div>
        <Icon className="h-5 w-5 text-primary" />
      </CardContent>
    </Card>
  );
}

export function DeveloperAppsPage() {
  const navigate = useNavigate();
  const { data, isLoading, error } = useDeveloperApps();
  const createMutation = useCreateDeveloperApp();
  const [createOpen, setCreateOpen] = useState(false);
  const [showInactive, setShowInactive] = useState(false);
  const [name, setName] = useState("");
  const [redirectUrisText, setRedirectUrisText] = useState("");
  const [clientType, setClientType] = useState<"public" | "confidential">(
    "public",
  );
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
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="font-display text-5xl font-normal tracking-tight">
            Developer Apps
          </h2>
          <p className="text-muted-foreground">
            Register and manage OAuth applications for your products.
          </p>
        </div>
        <div className="flex items-center gap-2">
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
                <Plus className="mr-2 h-4 w-4" />
                New Application
              </Button>
            </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Developer App</DialogTitle>
              <DialogDescription>
                Register a new OAuth application for your product.
              </DialogDescription>
            </DialogHeader>
            <div className="space-y-4">
              <div className="space-y-2">
                <label className="text-sm font-medium" htmlFor="app-name">
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
                <label className="text-sm font-medium" htmlFor="redirect-uris">
                  Redirect URIs (one per line)
                </label>
                <textarea
                  id="redirect-uris"
                  value={redirectUrisText}
                  onChange={(event) => setRedirectUrisText(event.target.value)}
                  placeholder={"https://app.example.com/oauth/callback\nmyapp://oauth/callback"}
                  className="flex min-h-[120px] w-full rounded-[10px] border border-input bg-transparent px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                />
              </div>
              <div className="space-y-2">
                <label className="text-sm font-medium">Client Type</label>
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
            </div>
            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => setCreateOpen(false)}
              >
                Cancel
              </Button>
              <Button onClick={() => void handleCreate()} isLoading={createMutation.isPending}>
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
          <Card className="xl:col-span-2">
            <CardContent className="p-8 text-center">
              <p className="text-sm text-muted-foreground">
                Failed to load developer apps. Please refresh and try again.
              </p>
            </CardContent>
          </Card>
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
                <CardTitle className="text-base">{app.client_name}</CardTitle>
                <Badge variant={app.is_active ? "success" : "secondary"}>
                  {app.is_active ? "active" : "inactive"}
                </Badge>
              </div>
              <CardDescription>
                Client ID:{" "}
                <span className="font-mono text-xs text-foreground">
                  {app.id}
                </span>
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{app.client_type}</Badge>
                <Badge variant="outline">
                  {app.redirect_uris.length} redirect URIs
                </Badge>
              </div>
              <div className="flex flex-wrap gap-2">
                {(app.allowed_scopes || "")
                  .split(/\s+/)
                  .filter(Boolean)
                  .map((scope) => (
                  <Badge key={scope} variant="secondary" className="text-xs">
                    {scope}
                  </Badge>
                  ))}
              </div>
              <div className="flex gap-2">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() =>
                    void navigate({
                      to: "/developer/apps/$clientId",
                      params: { clientId: app.id },
                    })
                  }
                >
                  View Details
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() =>
                    void navigate({
                      to: "/developer/apps/$clientId",
                      params: { clientId: app.id },
                    })
                  }
                >
                  Manage Credentials
                </Button>
              </div>
            </CardContent>
          </Card>
          ))}

        {!isLoading && visibleApps.length === 0 && (
          <Card className="xl:col-span-2">
            <CardContent className="p-8 text-center">
              <p className="text-sm text-muted-foreground">
                {apps.length === 0
                  ? "No developer apps yet. Create your first application."
                  : "No active applications. Enable 'Show inactive' to view deactivated apps."}
              </p>
            </CardContent>
          </Card>
        )}
      </div>
    </div>
  );
}
