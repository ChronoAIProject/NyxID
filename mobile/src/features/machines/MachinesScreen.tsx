import { useLayoutEffect, useMemo, useState } from "react";
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
  const runtime = useMachineRuntime();
  const selected =
    route.name === "MachineDetail" ? route.params.machineId : undefined;
  const tab = route.name === "Machines" ? route.params?.tab : undefined;
  const [handoffError, setHandoffError] = useState<string>();
  // Preserve the web query URL while detail gets real horizontal Back history.
  useLayoutEffect(() => {
    if (route.name === "Machines" && route.params?.machine) {
      const machineId = route.params.machine;
      navigation.setParams({ machine: undefined });
      navigation.push("MachineDetail", { machineId });
    }
  }, [navigation, route.name, route.params]);
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
    () => ({ title: node?.name ?? "Machines" }),
    [node?.name],
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
      if (runtime.current()) setHandoffError(machineError(cause));
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
      title={node?.name ?? "Machines"}
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
            accessibilityLabel="Loading machines"
          />
        ) : (
          <SkeletonRows
            rows={2}
            icon={false}
            accessibilityLabel="Loading machines"
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
              footer="Personal machines and organization machines you administer. Connectivity does not mean a desktop is live."
            >
              {inventory.data.nodes.map((machine) => (
                <SettingsRow
                  key={machine.id}
                  label={machine.name}
                  detail={`${machine.owner.display_name} · ${machineIsolationLabel(machine)}`}
                  value={machineConnectivity(machine)}
                  accessibilityLabel={`Details for ${machine.name}, ${machineConnectivity(machine)}, ${machine.owner.display_name}. ${machineIsolationLabel(machine)}. ${machine.machine?.os}/${machine.machine?.arch}, version ${machine.metadata?.agent_version ?? "unknown"}`}
                  onPress={() =>
                    navigation.push("MachineDetail", { machineId: machine.id })
                  }
                />
              ))}
              {!inventory.data.nodes.length && (
                <SettingsRow
                  label="No manageable machines"
                  detail="Pair a machine on the web to get started."
                />
              )}
            </SettingsGroup>
          )}
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label="Hosted Machines and Setup on Web"
              accessibilityHint="Opens NyxID on the web"
              onPress={() => void handoff(machineWebPath())}
            />
          </SettingsGroup>
        </>
      )}
      {selected && !inventory.isPending && !inventory.error && !node && (
        <>
          <Notice message="This machine is unavailable for native management. It may have been removed, transferred, hosted, or require organization Admin access." />
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label="Open This Machine on Web"
              accessibilityHint="Opens NyxID on the web"
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
  return on ? "Available" : "Off";
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
  const machine = node.machine!;
  const updateNotes = [update?.guidance, update?.updater_guidance]
    .filter(Boolean)
    .join("\n\n");
  return (
    <>
      <SettingsGroup inset={false}>
        <SettingsRow label="Status" value={machineConnectivity(node)} />
        <SettingsRow label="Owner" value={node.owner.display_name} />
        <SettingsRow
          label="Access"
          value={node.owner.kind === "org" ? "Organization Admin" : "Personal"}
        />
        <SettingsRow label="Platform" value={`${machine.os}/${machine.arch}`} />
        <SettingsRow
          label="Version"
          value={node.metadata?.agent_version ?? "Unknown"}
        />
      </SettingsGroup>
      <SettingsGroup title="Workspace roots" inset={false}>
        {machine.roots.length ? (
          machine.roots.map((root) => (
            <SettingsRow key={root} label={root} mono selectable />
          ))
        ) : (
          <SettingsRow label="None reported" />
        )}
      </SettingsGroup>
      <SettingsGroup
        title="Local capability ceiling"
        inset={false}
        footer="Configured on the machine. Agent permissions can only narrow this ceiling."
      >
        <SettingsRow label="Commands" value={availability(machine.shell)} />
        <SettingsRow label="Files" value={availability(machine.files)} />
        <SettingsRow
          label="Browser"
          value={availability(machine.browser ?? machine.computer)}
        />
        <SettingsRow label="Computer" value={availability(machine.computer)} />
      </SettingsGroup>
      <SettingsGroup title="Isolation" inset={false}>
        <SettingsRow
          label="Command isolation"
          value={
            machine.commands_isolated === true
              ? "Isolated"
              : machine.commands_isolated === false
                ? "Not isolated"
                : "Unknown; update to check"
          }
          valueTone={machine.commands_isolated === true ? "default" : "danger"}
        />
        <SettingsRow
          label="Browser isolation"
          value={machine.browser_isolated ? "Isolated" : "Not isolated"}
          valueTone={machine.browser_isolated ? "default" : "danger"}
        />
        <SettingsRow
          label="Computer display and permissions"
          value={machine.computer_ready ? "Ready" : "Unavailable"}
          valueTone={machine.computer_ready ? "default" : "danger"}
        />
      </SettingsGroup>
      {machine.shell && machine.commands_isolated === false && (
        <Notice message={SINGLE_USER_SHELL_WARNING} />
      )}
      <SettingsGroup
        title="Version and updates"
        inset={false}
        footer={updateNotes || undefined}
      >
        {update ? (
          <SettingsRow
            label="Update"
            value={`${update.current_version} → ${update.target_version}`}
          />
        ) : (
          <SettingsRow label="Update" value="Unavailable" />
        )}
        {update && (
          <SettingsRow
            label="Phase"
            value={`${update.phase}${update.code ? ` · ${update.code}` : ""}`}
          />
        )}
        <SettingsRow label="Owner Policy and Updates" onPress={onPolicy} />
      </SettingsGroup>
      <SettingsGroup inset={false}>
        <SettingsRow label="Machine Activity" onPress={onActivity} />
      </SettingsGroup>
      <MachineDesktopEntries nodeId={node.id} onOpen={onDesktop} />
    </>
  );
}
