import { useUiTranslation } from "@/i18n/use-ui-translation";
import { Badge } from "@/components/ui/badge";
import {
  SINGLE_USER_SHELL_WARNING,
  type MachineProfile,
} from "@/schemas/machines";

export function MachineIsolationBadge({
  machine,
}: {
  readonly machine: MachineProfile;
}) {
  const { t } = useUiTranslation();
  if (!machine.shell || machine.commands_isolated === true) return null;
  return (
    <Badge variant="warning">
      {machine.commands_isolated === false
        ? t("Not isolated")
        : t("Isolation unknown: update to check")}
    </Badge>
  );
}

export function MachineIsolationDetails({
  machine,
}: {
  readonly machine: MachineProfile;
}) {
  const { t } = useUiTranslation();
  if (!machine.shell || machine.commands_isolated !== false) return null;
  return (
    <div className="space-y-1 text-muted-foreground">
      <p>{t("Agent commands can read this node’s stored credentials and token.")}</p>
      <details>
        <summary className="cursor-pointer text-foreground">{t("Details")}</summary>
        <p className="pt-2">{SINGLE_USER_SHELL_WARNING}</p>
      </details>
    </div>
  );
}

/** Distinct from command-vs-node credential isolation. All current contexts share. */
export function MachineSharingBadge() {
  const { t } = useUiTranslation();
  return <Badge variant="warning" title={t("Agents granted this machine share its workspace and authenticated browser sessions.")}>{t("Shared workspace and browser")}</Badge>;
}
