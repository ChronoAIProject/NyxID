import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { ContentScreenProps } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { SegmentControl } from "../../components/SegmentControl";
import { SkeletonRows } from "../../components/Skeleton";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { usePullRefresh } from "../../hooks/usePullRefresh";
import { SavedLogins } from "./SavedLogins";
import { MachineDesktopEntries } from "./MachineDesktopEntries";
import {
  machineConnectivity,
  machineIsolationLabel,
  machinePageContent,
  machineTabs,
} from "./MachinePresentation";
import type { MachineDesktopParams } from "./desktop/desktopRoutes";
import { machinesApi } from "../../lib/api/machinesApi";
import {
  machineWebPath,
  SINGLE_USER_SHELL_WARNING,
  type MachineNode,
  type MachineUpdate,
} from "./machineContracts";
import {
  machineError,
  machineKeys,
  openMachineWeb,
  useMachineRuntime,
} from "./MachineUI";

export function MachinesScreen({
  route,
  navigation,
}: ContentScreenProps<"Machines"> | ContentScreenProps<"MachineDetail">) {
  const { t } = useAppLanguage();
  const runtime = useMachineRuntime();
  const selected =
    route.name === "MachineDetail" ? route.params.machineId : undefined;
  const tab = route.name === "Machines" ? route.params?.tab : undefined;
  const [handoffError, setHandoffError] = useLocalizedState<string>();
  // Preserve the web query URL while detail gets real horizontal Back history.
  useEffect(() => {
    if (route.name === "Machines" && route.params?.machine) {
      const machineId = route.params.machine;
      // Deep links commit the parent navigator after child layout effects.
      // Consume only the live request, including when effects are replayed.
      const current = navigation
        .getState()
        .routes.find((item) => item.key === route.key);
      if (
        current?.name !== "Machines" ||
        (current.params as { machine?: string } | undefined)?.machine !==
          machineId
      )
        return;
      navigation.setParams({ machine: undefined });
      navigation.push("MachineDetail", { machineId });
    }
  }, [navigation, route.key, route.name, route.params]);
  const inventory = useQuery({
    queryKey: [...machineKeys(runtime.epoch), "inventory"],
    queryFn: ({ signal }) => machinesApi.inventory(signal),
    enabled: runtime.active,
    retry: false,
    refetchInterval: runtime.active ? 10_000 : false,
    refetchIntervalInBackground: false,
  });
  const node = inventory.data?.nodes.find((row) => row.id === selected);
  const update = inventory.data?.updates.find(
    (row) => row.node_id === selected,
  );
  const headerOptions = useMemo(
    () => ({ title: node?.name ?? t("Machines") }),
    [node?.name, t],
  );
  useWorkspaceBack({
    active: false,
    defaultRole: selected ? "back" : "menu",
    onBack: navigation.goBack,
    options: headerOptions,
  });
  function activity(machine: MachineNode) {
    navigation.navigate("MachineActivity", {
      machineId: machine.id,
      ownerId: machine.owner.id,
    });
  }
  async function handoff(path: string) {
    try {
      await openMachineWeb(path);
    } catch (cause) {
      if (runtime.current()) setHandoffError(() => (machineError(cause)));
    }
  }
  const pull = usePullRefresh(async () => {
    if (!runtime.active || inventory.isFetching) return;
    await inventory.refetch();
  });
  if (!selected && tab === "logins") {
    return (
      <SavedLogins
        inventory={inventory.data}
        handoffError={handoffError}
        onWeb={(path) => void handoff(path)}
        onMachines={() => navigation.setParams({ tab: undefined })}
        onOpen={(login, ownerLabel) =>
          navigation.push("MachineSavedLogin", {
            loginId: login.id,
            ownerLabel,
          })
        }
      />
    );
  }
  return (
    <WorkspacePage
      title={node?.name ?? translateApp("Machines")}
      showHeading={false}
      refreshing={pull.refreshing}
      onRefresh={pull.onRefresh}
      contentStyle={machinePageContent}
    >
      {!selected && (
        <SegmentControl
          segments={machineTabs}
          activeIndex={0}
          onPress={(index) => {
            if (index === 1) navigation.setParams({ tab: "logins" });
          }}
        />
      )}
      {handoffError && <Notice tone="error" message={handoffError} />}
      {inventory.isPending &&
        (selected ? (
          <SkeletonRows
            rows={4}
            icon={false}
            detail={false}
            accessibilityLabel={translateApp("Loading machines")}
          />
        ) : (
          <SkeletonRows
            rows={2}
            icon={false}
            accessibilityLabel={translateApp("Loading machines")}
          />
        ))}
      {inventory.error && (
        <Notice tone="error" message={machineError(inventory.error)} />
      )}
      {!selected && (
        <>
          {inventory.data && !inventory.error && (
            <SettingsGroup
              inset={false}
              footer={translateApp(
                "Personal machines and organization machines you administer. Connectivity does not mean a desktop is live.",
              )}
            >
              {inventory.data.nodes.map((machine) => (
                <SettingsRow
                  key={machine.id}
                  label={machine.name}
                  detail={`${machine.owner.display_name} · ${machineIsolationLabel(machine)}`}
                  value={machineConnectivity(machine)}
                  accessibilityLabel={translateApp(
                    "Details for {{name}}, {{connectivity}}, {{owner}}. {{isolation}}. {{os}}/{{architecture}}, version {{version}}",
                    {
                      name: machine.name,
                      connectivity: machineConnectivity(machine),
                      owner: machine.owner.display_name,
                      isolation: machineIsolationLabel(machine),
                      os: machine.machine?.os,
                      architecture: machine.machine?.arch,
                      version: machine.metadata?.agent_version ?? "unknown",
                    },
                  )}
                  onPress={() =>
                    navigation.push("MachineDetail", { machineId: machine.id })
                  }
                />
              ))}
              {!inventory.data.nodes.length && (
                <SettingsRow
                  label={translateApp("No manageable machines")}
                  detail={translateApp(
                    "Pair a machine on the web to get started.",
                  )}
                />
              )}
            </SettingsGroup>
          )}
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label={translateApp("Hosted Machines and Setup on Web")}
              accessibilityHint={translateApp("Opens NyxID on the web")}
              onPress={() => void handoff(machineWebPath())}
            />
          </SettingsGroup>
        </>
      )}
      {selected && !inventory.isPending && !inventory.error && !node && (
        <>
          <Notice message={translateApp(
              "This machine is unavailable for native management. It may have been removed, transferred, hosted, or require organization Admin access.",
            )}
          />
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label={translateApp("Open This Machine on Web")}
              accessibilityHint={translateApp("Opens NyxID on the web")}
              onPress={() => void handoff(machineWebPath(selected))}
            />
          </SettingsGroup>
        </>
      )}
      {node && !inventory.error && (
        <MachineDetails
          node={node}
          update={update}
          onPolicy={() =>
            navigation.navigate("MachinePolicy", {
              machineId: node.id,
              ownerId: node.owner.id,
            })
          }
          onActivity={() => activity(node)}
          onDesktop={(params) => navigation.navigate("MachineDesktop", params)}
        />
      )}
    </WorkspacePage>
  );
}

function availability(on: boolean | null | undefined) {
  return on ? translateApp("Available") : translateApp("Off");
}

export function MachineDetails({
  node,
  update,
  onPolicy,
  onActivity,
  onDesktop,
}: {
  node: MachineNode;
  update?: MachineUpdate;
  onPolicy: () => void;
  onActivity: () => void;
  onDesktop: (params: MachineDesktopParams) => void;
}) {
  useAppLanguage();
  const machine = node.machine!;
  const updateNotes = [update?.guidance, update?.updater_guidance]
    .filter(Boolean)
    .join("\n\n");
  return (
    <>
      <SettingsGroup inset={false}>
        <SettingsRow label={translateApp("Status")}
          value={machineConnectivity(node)} />
        <SettingsRow label={translateApp("Owner")}
          value={node.owner.display_name} />
        <SettingsRow
          label={translateApp("Access")}
          value={node.owner.kind === "org" ? translateApp("Organization Admin")
              : translateApp("Personal")
          }
        />
        <SettingsRow label={translateApp("Platform")}
          value={`${machine.os}/${machine.arch}`} />
        <SettingsRow
          label={translateApp("Version")}
          value={node.metadata?.agent_version ?? translateApp("Unknown")}
        />
      </SettingsGroup>
      <SettingsGroup title={translateApp("Workspace roots")} inset={false}>
        {machine.roots.length ? (
          machine.roots.map((root) => (
            <SettingsRow key={root} label={root} mono selectable />
          ))
        ) : (
          <SettingsRow label={translateApp("None reported")} />
        )}
      </SettingsGroup>
      <SettingsGroup
        title={translateApp("Local capability ceiling")}
        inset={false}
        footer={translateApp(
          "Configured on the machine. Agent permissions can only narrow this ceiling.",
        )}
      >
        <SettingsRow label={translateApp("Commands")}
          value={availability(machine.shell)} />
        <SettingsRow label={translateApp("Files")}
          value={availability(machine.files)} />
        <SettingsRow
          label={translateApp("Browser")}
          value={availability(machine.browser ?? machine.computer)}
        />
        <SettingsRow label={translateApp("Computer")}
          value={availability(machine.computer)} />
      </SettingsGroup>
      <SettingsGroup title={translateApp("Isolation")} inset={false}>
        <SettingsRow
          label={translateApp("Command isolation")}
          value={
            machine.commands_isolated === true
              ? translateApp("Isolated")
              : machine.commands_isolated === false
                ? translateApp("Not isolated")
                : translateApp("Unknown; update to check")
          }
          valueTone={machine.commands_isolated === true ? "default" : "danger"}
        />
        <SettingsRow
          label={translateApp("Browser isolation")}
          value={machine.browser_isolated ? translateApp("Isolated")
              : translateApp("Not isolated")
          }
          valueTone={machine.browser_isolated ? "default" : "danger"}
        />
        <SettingsRow
          label={translateApp("Computer display and permissions")}
          value={machine.computer_ready ? translateApp("Ready")
              : translateApp("Unavailable")
          }
          valueTone={machine.computer_ready ? "default" : "danger"}
        />
      </SettingsGroup>
      {machine.shell && machine.commands_isolated === false && (
        <Notice message={translateApp(SINGLE_USER_SHELL_WARNING)} />
      )}
      <SettingsGroup
        title={translateApp("Version and updates")}
        inset={false}
        footer={updateNotes || undefined}
      >
        {update ? (
          <SettingsRow
            label={translateApp("Update")}
            value={`${update.current_version} → ${update.target_version}`}
          />
        ) : (
          <SettingsRow label={translateApp("Update")}
            value={translateApp("Unavailable")}
          />
        )}
        {update && (
          <SettingsRow
            label={translateApp("Phase")}
            value={`${update.phase}${update.code ? ` · ${update.code}` : ""}`}
          />
        )}
        <SettingsRow label={translateApp("Owner Policy and Updates")}
          onPress={onPolicy} />
      </SettingsGroup>
      <SettingsGroup inset={false}>
        <SettingsRow label={translateApp("Machine Activity")}
          onPress={onActivity} />
      </SettingsGroup>
      <MachineDesktopEntries nodeId={node.id} onOpen={onDesktop} />
    </>
  );
}
