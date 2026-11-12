import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { Badge } from "@/components/ui/badge";
import {
  SINGLE_USER_SHELL_WARNING,
  machineSeparationSupport,
  type MachineProfile,
} from "@/schemas/machines";
import { SEPARATED_SETUP_NOTE } from "@/schemas/machine-access";

export function MachineIsolationBadge({
  machine,
}: {
  readonly machine: MachineProfile;
}) {
  useUiTranslation();
  if (!machine.shell || machine.commands_isolated === true) return null;
  return (
    <Badge variant="warning">
      {machine.commands_isolated === false
        ? translateUi("Not isolated")
        : translateUi("Isolation unknown: update to check")}
    </Badge>
  );
}

export function MachineIsolationDetails({
  machine,
}: {
  readonly machine: MachineProfile;
}) {
  useUiTranslation();
  if (!machine.shell || machine.commands_isolated !== false) return null;
  return (
    <div className="space-y-1 text-muted-foreground">
      <p>{translateUi("Agent commands can read this node’s stored credentials and token.")}</p>
      <details>
        <summary className="cursor-pointer text-foreground">{translateUi("Details")}</summary>
        <p className="pt-2">{SINGLE_USER_SHELL_WARNING}</p>
      </details>
    </div>
  );
}

/** A node can have both legacy and separated assignments. */
export function MachineSharingBadge() {
  useUiTranslation();
  return <Badge variant="secondary" title={translateUi("Review each agent’s assignment. Shared legacy agents share workspace and authenticated browser sessions; separated assignments use their own workspace and browser profiles.")}>{translateUi("Sharing set per agent")}</Badge>;
}

const supportReasons: Record<string, string> = {
  separated_browser_sandbox_unobservable_in_container: "This container does not expose every kernel observation required to confirm the managed browser sandbox. Separated contexts are unavailable. Use a supported native Linux installation or a separate machine container or VM per agent; do not add tracing capabilities or disable the browser sandbox.",
  node_custody_unverified: "Credential custody could not be verified for all command and browser users. Upgrade or repair the node’s protected credential storage and rerun setup. Separation remains unavailable until real identity checks confirm that node tokens, signing secrets and stored credentials are protected.",
  separated_browser_policy_not_effective: "The actual managed secure browser did not confirm that its policy blocks DevTools. Reinstall managed policies for this executable and rerun setup. Separation remains unavailable until the live policy check succeeds.",
  separated_requires_linux: "This runtime supports per-agent separation only on Linux. macOS has no supported per-agent sandbox. Use a supported Linux host.",
  separated_requires_landlock_abi_6: "Working Landlock ABI 6 or newer is required. Upgrade the Linux kernel or use a supported host.",
  separated_requires_separate_users: "A root supervisor and distinct non-root command, secure-browser and developer-browser users are required. On Linux, rerun setup with sudo and --separate-users.",
  separated_requires_managed_browser: "A managed Chromium installation is required. Browserless nodes cannot create separated contexts; use a separate container per agent or configure a supported Linux browser installation.",
  separated_seccomp_probe_failed: "The command user’s seccomp policy could not be confirmed. Check the host security policy and rerun setup.",
  separated_landlock_probe_failed: "Landlock enforcement could not be confirmed. Check the host security policy and rerun setup.",
  separated_browser_sandbox_probe_failed: "The actual managed Chromium sandbox could not be confirmed under both browser users. Check its executable and AppArmor user-namespace policy before rerunning setup.",
};

export function MachineSeparationSupport({ machine, support, setupNote, humanOnly = false }: {
  readonly machine?: MachineProfile;
  readonly support?: { available: boolean; reason: string | null } | null;
  readonly setupNote?: string | null;
  readonly humanOnly?: boolean;
}) {
  useUiTranslation();
  // Access DTOs contain the server's current selector result; raw profiles need
  // evidence validation, because their legacy support boolean is insufficient.
  const current = machine ? machineSeparationSupport(machine) : support;
  const reason = current?.reason;
  const confirmed = current?.available === true && !reason;
  const timeout = reason === "separated_probe_timeout";
  const unsupported = !!reason && Object.hasOwn(supportReasons, reason);
  const label = confirmed
    ? "Separation support confirmed"
    : timeout ? "Separation probe timed out"
    : unsupported ? "Separation prerequisites not met"
    : "Separation support unknown";
  return <div className={humanOnly ? "space-y-1 text-13 text-text-tertiary" : "space-y-1 text-14 text-muted-foreground"} aria-label={translateUi("Separation support")} data-support-reason={reason}>
    {humanOnly ? <p className={confirmed ? "text-foreground" : "text-warning"}>{label}</p> : <Badge variant={confirmed ? "success" : "warning"}>{label}</Badge>}
    <p>{confirmed
      ? translateUi("The latest capability report confirms support. Each agent’s stored mode and permissions still apply; existing shared legacy assignments stay shared.")
      : timeout
        ? translateUi("A probe exceeded its five-second bound. A cold Chromium start can cause this; rerun setup when the host is less busy. Support remains unconfirmed until every probe succeeds.")
        : unsupported ? supportReasons[reason]
        : translateUi("Complete supported capability evidence is missing. Upgrade or reconnect the node and rerun setup. An older support boolean cannot confirm a new separated assignment.")}</p>
    {!confirmed ? <p>{setupNote ?? SEPARATED_SETUP_NOTE}</p> : null}
    {reason && !humanOnly ? <details><summary className="cursor-pointer">{translateUi("Separate workspace diagnostics")}</summary><p>{reason}</p>{machine?.separated?.reason && machine.separated.reason !== reason ? <p>{machine.separated.reason}</p> : null}</details> : null}
  </div>;
}
