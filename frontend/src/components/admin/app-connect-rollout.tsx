import { toast } from "sonner";
import {
  useAppConnectRollout,
  useUpdateAppConnectRollout,
  useUpdateAppConnectCapability,
} from "@/hooks/use-app-requirements";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";

export function AppConnectRolloutPolicy() {
  const { data, isPending, error } = useAppConnectRollout(true);
  const mutation = useUpdateAppConnectRollout();
  async function setRollout(rollout: "disabled" | "allowlist" | null) {
    try {
      await mutation.mutateAsync(rollout);
      toast.success("App Connect rollout updated");
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Could not update rollout",
      );
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>App Connect Rollout Policy</CardTitle>
        <CardDescription>
          Requirements are available only to apps with an admin-granted
          capability. Allowlist mode also requires ownership by a configured
          organization.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {isPending && (
          <p className="text-sm text-muted-foreground">Loading policy…</p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error.message}
          </p>
        )}
        {data && (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant="secondary">{data.effective}</Badge>
              <span className="text-xs text-muted-foreground">
                {data.override_value === null
                  ? "Environment default"
                  : "Runtime override"}{" "}
                · Default: {data.env_default} · {data.allowed_org_ids.length}{" "}
                allowed organizations
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                variant="outline"
                disabled={mutation.isPending || data.effective === "disabled"}
                onClick={() => void setRollout("disabled")}
              >
                Disable
              </Button>
              <Button
                variant="outline"
                disabled={mutation.isPending || data.effective === "allowlist"}
                onClick={() => void setRollout("allowlist")}
              >
                Use allowlist
              </Button>
              {data.override_value !== null && (
                <Button
                  variant="ghost"
                  disabled={mutation.isPending}
                  onClick={() => void setRollout(null)}
                >
                  Reset to environment default
                </Button>
              )}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}

export function AppConnectCapabilitySwitch({
  clientId,
  clientName,
  enabled,
}: {
  readonly clientId: string;
  readonly clientName: string;
  readonly enabled: boolean;
}) {
  const mutation = useUpdateAppConnectCapability();
  async function update(checked: boolean) {
    try {
      await mutation.mutateAsync({ clientId, enabled: checked });
      toast.success("App Connect capability updated");
    } catch (err) {
      toast.error(
        err instanceof Error ? err.message : "Could not update capability",
      );
    }
  }
  return (
    <label className="flex items-center gap-2 text-xs text-muted-foreground">
      <Switch
        aria-label={`App Connect capability for ${clientName} (${clientId})`}
        checked={enabled}
        disabled={mutation.isPending}
        onCheckedChange={(checked) => void update(checked)}
      />
      App Connect
    </label>
  );
}
