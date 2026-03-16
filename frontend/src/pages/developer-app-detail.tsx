import { useState } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
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
import { parseRedirectUris } from "@/lib/oauth";
import { ApiError } from "@/lib/api-client";
import { toast } from "sonner";

const OIDC_SCOPES = [
  { id: "openid", label: "openid", required: true },
  { id: "profile", label: "profile", required: false },
  { id: "email", label: "email", required: false },
  { id: "roles", label: "roles", required: false, hint: "Includes user roles and permissions in tokens" },
  { id: "groups", label: "groups", required: false, hint: "Includes user group memberships in tokens" },
] as const;

export function DeveloperAppDetailPage() {
  const navigate = useNavigate();
  const { clientId } = useParams({ strict: false }) as { clientId: string };
  const { data: app, isLoading } = useDeveloperApp(clientId);
  const updateMutation = useUpdateDeveloperApp();
  const deleteMutation = useDeleteDeveloperApp();
  const rotateMutation = useRotateDeveloperAppSecret();

  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [name, setName] = useState("");
  const [redirectUrisText, setRedirectUrisText] = useState("");
  const [editScopes, setEditScopes] = useState<readonly string[]>([]);

  const [secretOpen, setSecretOpen] = useState(false);
  const [rotatedSecret, setRotatedSecret] = useState("");

  function openEditDialog() {
    if (!app) return;
    setName(app.client_name);
    setRedirectUrisText(app.redirect_uris.join("\n"));
    setEditScopes(app.allowed_scopes.split(/\s+/).filter(Boolean));
    setEditOpen(true);
  }

  async function handleSave() {
    if (!app) return;
    const parsedUris = parseRedirectUris(redirectUrisText);

    if (!name.trim()) {
      toast.error("Name is required");
      return;
    }

    if (parsedUris.error) {
      toast.error(parsedUris.error);
      return;
    }

    try {
      await updateMutation.mutateAsync({
        clientId: app.id,
        data: {
          name: name.trim(),
          redirect_uris: parsedUris.uris,
          allowed_scopes: editScopes,
        },
      });
      toast.success("Application updated");
      setEditOpen(false);
    } catch (error) {
      toast.error(error instanceof ApiError ? error.message : "Update failed");
    }
  }

  async function handleRotateSecret() {
    if (!app) return;
    try {
      const result = await rotateMutation.mutateAsync(app.id);
      setRotatedSecret(result.client_secret);
      setSecretOpen(true);
      toast.success("Client secret rotated");
    } catch (error) {
      toast.error(error instanceof ApiError ? error.message : "Rotate failed");
    }
  }

  async function handleDelete() {
    if (!app) return;
    try {
      await deleteMutation.mutateAsync(app.id);
      toast.success("Application deactivated");
      setDeleteOpen(false);
      void navigate({ to: "/developer/apps" });
    } catch (error) {
      toast.error(error instanceof ApiError ? error.message : "Delete failed");
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
        <h2 className="font-display text-3xl md:text-5xl font-normal tracking-tight">
          Developer App
        </h2>
        <p className="text-muted-foreground">
          App not found or you do not have access.
        </p>
        <Button variant="outline" onClick={() => void navigate({ to: "/developer/apps" })}>
          Back to Apps
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <h2 className="font-display text-3xl font-normal tracking-tight md:text-5xl">
            {app.client_name}
          </h2>
          <p className="text-sm text-muted-foreground">
            Client details, redirect URIs, and OAuth metadata.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" onClick={() => void navigate({ to: "/developer/apps" })}>
            Back
          </Button>
          <Button variant="outline" onClick={openEditDialog}>
            Edit
          </Button>
          {app.client_type === "confidential" && (
            <Button variant="outline" onClick={() => void handleRotateSecret()}>
              Rotate Secret
            </Button>
          )}
          <Button variant="destructive" onClick={() => setDeleteOpen(true)}>
            Deactivate
          </Button>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Credentials</CardTitle>
          <CardDescription>
            These values identify your OAuth client.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-2">
            <Badge variant="outline">{app.client_type}</Badge>
            <Badge variant={app.is_active ? "success" : "secondary"}>
              {app.is_active ? "active" : "inactive"}
            </Badge>
          </div>
          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">Client ID</p>
            <p className="break-all font-mono text-sm text-foreground">{app.id}</p>
          </div>
          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">Created At</p>
            <p className="text-sm text-foreground">
              {new Date(app.created_at).toLocaleString()}
            </p>
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Redirect URIs</CardTitle>
          <CardDescription>
            OAuth callback destinations configured for this client.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {app.redirect_uris.map((uri) => (
            <div
              key={uri}
              className="break-all rounded-md border border-border bg-muted px-3 py-2 font-mono text-xs"
            >
              {uri}
            </div>
          ))}
          {app.redirect_uris.length === 0 && (
            <p className="text-sm text-muted-foreground">No redirect URIs configured.</p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Allowed Scopes</CardTitle>
          <CardDescription>
            OIDC scopes this client can request. Determines what user data is included in tokens.
          </CardDescription>
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

      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Edit Developer App</DialogTitle>
            <DialogDescription>
              Update application name and redirect URIs.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
            <div className="space-y-2">
              <label className="text-sm font-medium" htmlFor="edit-app-name">
                Application Name
              </label>
              <Input
                id="edit-app-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
              />
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium" htmlFor="edit-redirect-uris">
                Redirect URIs (one per line)
              </label>
              <textarea
                id="edit-redirect-uris"
                value={redirectUrisText}
                onChange={(event) => setRedirectUrisText(event.target.value)}
                className="flex min-h-[120px] w-full rounded-[10px] border border-input bg-transparent px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
              />
            </div>
            <div className="space-y-3">
              <label className="text-sm font-medium">Allowed Scopes</label>
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
                        className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70"
                      >
                        {scope.label}
                        {scope.required && (
                          <span className="ml-1 text-xs text-muted-foreground">(required)</span>
                        )}
                      </label>
                      {"hint" in scope && scope.hint && (
                        <p className="text-xs text-muted-foreground">{scope.hint}</p>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditOpen(false)}>
              Cancel
            </Button>
            <Button onClick={() => void handleSave()} isLoading={updateMutation.isPending}>
              Save
            </Button>
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
            <DialogTitle>Deactivate Application</DialogTitle>
            <DialogDescription>
              This will disable OAuth authorization for this app. Existing
              tokens may stop working depending on downstream policy.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDeleteOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => void handleDelete()}
              isLoading={deleteMutation.isPending}
            >
              Deactivate
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
