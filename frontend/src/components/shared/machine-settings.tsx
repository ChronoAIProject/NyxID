import { useState } from "react";
import { useAppForm } from "@/components/ui/form";
import { Link } from "@tanstack/react-router";
import { useMachineSettings } from "@/hooks/use-machines";
import {
  useNyxBotAgents,
  useSetNyxBotAgentGrants,
} from "@/hooks/use-nyxbot-agents";
import {
  MACHINE_SAFETY,
  SINGLE_USER_WARNING,
  SINGLE_USER_SHELL_WARNING,
} from "@/schemas/machines";
import type { NodeInfo } from "@/types/nodes";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { DetailSection } from "@/components/shared/detail-section";

export function MachineSettings({
  node,
  canManage,
}: {
  readonly node: NodeInfo;
  readonly canManage: boolean;
}) {
  const settings = useMachineSettings(node.id);
  const agents = useNyxBotAgents();
  const grants = useSetNyxBotAgentGrants();
  const form = useAppForm({
    defaultValues: {
      confirm: node.machine_confirm ?? "none",
      allow: node.allow_single_user_saved_logins ?? false,
    },
  });
  const confirm = form.watch("confirm");
  const allow = form.watch("allow");
  const [acknowledged, setAcknowledged] = useState(false);
  const machine = node.machine;
  if (!machine) return null;
  const changed =
    confirm !== (node.machine_confirm ?? "none") ||
    allow !== (node.allow_single_user_saved_logins ?? false);
  return (
    <DetailSection title="Machine access">
      <div className="max-w-2xl space-y-4 text-12">
        <p>
          {[
            machine.shell && "Commands",
            machine.files && "Files",
            machine.computer && "Computer",
          ]
            .filter(Boolean)
            .join(" · ")}{" "}
          · {machine.os} / {machine.arch}
        </p>
        {machine.shell && !machine.browser_isolated ? (
          <div className="space-y-2">
            <Badge variant="warning">Not isolated</Badge>
            <p className="text-muted-foreground">{SINGLE_USER_SHELL_WARNING}</p>
          </div>
        ) : null}
        <p>Workspace roots: {machine.roots.join(", ")}</p>
        {machine.computer ? (
          <p>
            Computer mode: {machine.computer_mode}. Driver:{" "}
            {machine.cua_version ?? "unavailable"}.{" "}
            {machine.computer_ready
              ? "Ready"
              : "Permissions or display unavailable"}
            .
          </p>
        ) : null}
        {machine.computer_permissions ? (
          <p>
            Screen Recording:{" "}
            {machine.computer_permissions.screen_recording
              ? "allowed"
              : "not granted"}
            . Accessibility:{" "}
            {machine.computer_permissions.accessibility
              ? "allowed"
              : "not granted"}
            . Enable permissions for the app running the node in macOS System
            Settings, then restart the node.
          </p>
        ) : null}
        {machine.computer ? (
          <Button asChild>
            <Link
              to="/assistant/machines/$nodeId/desktop"
              params={{ nodeId: node.id }}
              search={{ conversation_id: undefined }}
            >
              Open live desktop
            </Link>
          </Button>
        ) : null}
        {canManage ? (
          <fieldset
            className="space-y-2"
            disabled={!canManage || grants.isPending}
          >
            <legend className="mb-2 font-medium">Specialist access</legend>
            <p className="text-muted-foreground">
              NyxBot can use every machine you own or administer.
            </p>
            {agents.isPending ? (
              <p role="status">Loading specialists…</p>
            ) : null}
            {agents.data?.agents
              .filter(
                (agent) =>
                  agent.kind !== "nyxbot" && agent.status !== "destroyed",
              )
              .map((agent) => (
                <label key={agent.id} className="flex items-center gap-2">
                  <Checkbox
                    checked={agent.machines?.includes(node.id) ?? false}
                    onCheckedChange={(checked) =>
                      grants.mutate({
                        id: agent.id,
                        services: agent.services,
                        account_read: agent.account_read,
                        machines:
                          checked === true
                            ? [...new Set([...(agent.machines ?? []), node.id])]
                            : (agent.machines ?? []).filter(
                                (id) => id !== node.id,
                              ),
                      })
                    }
                  />
                  {agent.display_name ?? agent.name}
                </label>
              ))}
            {agents.error || grants.error ? (
              <p role="alert" className="text-destructive">
                {agents.error?.message ?? grants.error?.message}
              </p>
            ) : null}
          </fieldset>
        ) : null}
        <p className="text-muted-foreground">{MACHINE_SAFETY}</p>
        {canManage ? (
          <>
            <label className="block space-y-2">
              Owner confirmation
              <Select
                value={confirm}
                onValueChange={(value) =>
                  form.setValue("confirm", value as typeof confirm)
                }
              >
                <SelectTrigger aria-label="Owner confirmation">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">No confirmation</SelectItem>
                  <SelectItem value="changes">Confirm changes</SelectItem>
                  <SelectItem value="all">Confirm every operation</SelectItem>
                </SelectContent>
              </Select>
            </label>
            {machine.computer && !machine.browser_isolated ? (
              <div className="space-y-3 rounded-lg border border-warning/30 p-4">
                <p>{SINGLE_USER_WARNING}</p>
                <label className="flex items-center gap-2">
                  <Checkbox
                    checked={allow}
                    onCheckedChange={(value) => {
                      form.setValue("allow", value === true);
                      setAcknowledged(false);
                    }}
                  />
                  Allow saved-login typing on this machine
                </label>
                {allow && !node.allow_single_user_saved_logins ? (
                  <label className="flex items-center gap-2">
                    <Checkbox
                      checked={acknowledged}
                      onCheckedChange={(value) =>
                        setAcknowledged(value === true)
                      }
                    />
                    I understand the shared-user risk and allow typing.
                  </label>
                ) : null}
              </div>
            ) : null}
            <Button
              variant="primary"
              disabled={
                !changed ||
                (allow && !node.allow_single_user_saved_logins && !acknowledged)
              }
              isLoading={settings.isPending}
              onClick={() =>
                settings.mutate({
                  machine_confirm: confirm,
                  allow_single_user_saved_logins: allow,
                  acknowledge_single_user_risk: acknowledged,
                })
              }
            >
              Save machine settings
            </Button>
            {settings.error ? (
              <p role="alert" className="text-destructive">
                {settings.error.message}
              </p>
            ) : null}
          </>
        ) : null}
        <p>
          <Link to="/assistant/machines" search={{ tab: "logins" }}>
            Manage saved logins
          </Link>
        </p>
      </div>
    </DetailSection>
  );
}
