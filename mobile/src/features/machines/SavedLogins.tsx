import { useState } from "react";
import { Alert } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { ContentScreenProps } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { SegmentControl } from "../../components/SegmentControl";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import {
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { machinesApi } from "../../lib/api/machinesApi";
import {
  machineWebPath,
  type MachineInventory,
  type SavedLogin,
} from "./machineContracts";
import {
  machineError,
  machineKeys,
  useMachineRuntime,
  useMachineWrite,
} from "./MachineUI";
import {
  formatMachineDate,
  machinePageContent,
  machineTabs,
} from "./MachinePresentation";

const loginsKey = (epoch: number) => [...machineKeys(epoch), "logins"];

export function savedLoginOwnerLabel(
  login: SavedLogin,
  inventory?: MachineInventory,
) {
  return login.owner_id === inventory?.userId
    ? "Your personal account"
    : (inventory?.nodes.find((node) => node.owner.id === login.owner_id)?.owner
        .display_name ?? `Organization ${login.owner_id}`);
}

/** Metadata summary only: whether secrets exist, never their values. */
export function savedLoginSecretsSummary(login: SavedLogin) {
  if (login.has_password && login.has_totp) return "Password + TOTP";
  if (login.has_password) return "Password";
  return login.has_totp ? "TOTP" : "No secrets";
}

export function SavedLogins({
  inventory,
  handoffError,
  onWeb,
  onMachines,
  onOpen,
}: {
  inventory?: MachineInventory;
  handoffError?: string;
  onWeb: (path: string) => void;
  onMachines: () => void;
  onOpen: (login: SavedLogin, ownerLabel: string) => void;
}) {
  const runtime = useMachineRuntime();
  const [refreshError, setRefreshError] = useState<string>();
  const logins = useQuery({
    queryKey: loginsKey(runtime.epoch),
    queryFn: ({ signal }) => machinesApi.logins(signal),
    enabled: runtime.active,
    retry: false,
  });
  async function refresh() {
    const result = await logins.refetch();
    if (!runtime.current()) return;
    setRefreshError(result.error ? machineError(result.error) : undefined);
  }
  return (
    <WorkspacePage
      title="Saved logins"
      showHeading={false}
      refreshing={logins.isRefetching}
      onRefresh={() => {
        if (runtime.active) void refresh();
      }}
      contentStyle={machinePageContent}
    >
      <SegmentControl
        segments={machineTabs}
        activeIndex={1}
        onPress={(index) => {
          if (index === 0) onMachines();
        }}
      />
      {handoffError && <Notice tone="error" message={handoffError} />}
      {logins.isPending && <Label muted>Loading saved logins…</Label>}
      {logins.error && (
        <Notice tone="error" message={machineError(logins.error)} />
      )}
      {refreshError && <Notice tone="error" message={refreshError} />}
      {logins.data && !logins.error && (
        <SettingsGroup
          inset={false}
          footer="Metadata only. Passwords and TOTP secrets are never returned or stored by this screen. Create or edit logins on the web."
        >
          {logins.data.map((login) => {
            const owner = savedLoginOwnerLabel(login, inventory);
            return (
              <SettingsRow
                key={login.id}
                label={login.label}
                detail={[owner, login.username_hint]
                  .filter(Boolean)
                  .join(" · ")}
                value={savedLoginSecretsSummary(login)}
                accessibilityLabel={`${login.label}, ${owner}. ${savedLoginSecretsSummary(login)}`}
                onPress={() => onOpen(login, owner)}
              />
            );
          })}
          {!logins.data.length && (
            <SettingsRow label="No saved logins available" />
          )}
        </SettingsGroup>
      )}
      <SettingsGroup inset={false}>
        <SettingsRow
          tone="action"
          label="Create or Edit Saved Logins on Web"
          accessibilityHint="Opens NyxID on the web"
          onPress={() => onWeb(machineWebPath(undefined, "logins"))}
        />
      </SettingsGroup>
    </WorkspacePage>
  );
}

/** One saved login's metadata and its irreversible deletion. */
export function SavedLoginScreen({
  route,
  navigation,
}: ContentScreenProps<"MachineSavedLogin">) {
  const write = useMachineWrite();
  // A deletion in flight keeps the page so its outcome is never lost.
  useWorkspaceBack({
    active: write.busy,
    defaultRole: "back",
    disabled: write.busy,
    onBack: navigation.goBack,
  });
  const client = useQueryClient();
  const [notice, setNotice] = useState<string>();
  const [refreshError, setRefreshError] = useState<string>();
  const logins = useQuery({
    queryKey: loginsKey(write.epoch),
    queryFn: ({ signal }) => machinesApi.logins(signal),
    enabled: write.active,
    retry: false,
  });
  const login = logins.data?.find((row) => row.id === route.params.loginId);
  const owner = route.params.ownerLabel;
  const blocked =
    write.busy || write.uncertain || !write.active || !!logins.error;
  async function refresh() {
    const result = await logins.refetch();
    if (!write.current()) return;
    if (result.error) setRefreshError(machineError(result.error));
    else {
      write.reviewed();
      setRefreshError(undefined);
    }
  }
  function remove(target: SavedLogin) {
    void write.run(async (current) => {
      await machinesApi.removeLogin(target, current);
      if (!current()) return;
      setNotice(
        "Saved login deleted. Its login contexts are quarantined and specialist grants have been removed.",
      );
      await Promise.all([
        client.invalidateQueries({ queryKey: machineKeys(write.epoch) }),
        client.invalidateQueries({ queryKey: ["workspace", "agents"] }),
      ]);
    });
  }
  function review(target: SavedLogin) {
    if (blocked) return;
    Alert.alert(
      `Delete ${target.label} from ${owner}?`,
      "This permanently removes the saved login, quarantines its login contexts, and removes specialist login grants. Existing external sign-ins may remain active. This cannot be undone.",
      [
        { text: "Keep saved login", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: () => remove(target),
        },
      ],
    );
  }
  return (
    <WorkspacePage
      title={login?.label ?? "Saved login"}
      showHeading={false}
      refreshing={logins.isRefetching}
      onRefresh={() => {
        if (!write.busy && write.active) void refresh();
      }}
      contentStyle={machinePageContent}
    >
      {logins.isPending && <Label muted>Loading saved login…</Label>}
      {logins.error && (
        <Notice tone="error" message={machineError(logins.error)} />
      )}
      {write.error && <Notice tone="error" message={write.error} />}
      {refreshError && <Notice tone="error" message={refreshError} />}
      {notice && <Notice tone="success" message={notice} />}
      {logins.data && !login && !notice && (
        <Notice message="This saved login is no longer available. It may have been deleted on another device." />
      )}
      {login && (
        <>
          <SettingsGroup inset={false}>
            <SettingsRow label="Name" value={login.label} />
            <SettingsRow label="Owner" value={owner} />
            {login.username_hint ? (
              <SettingsRow label="Username" value={login.username_hint} />
            ) : null}
            <SettingsRow
              label="Sign-in confirmation"
              value={
                login.confirm_each_sign_in
                  ? "Every sign-in"
                  : "Machine confirmation"
              }
            />
          </SettingsGroup>
          <SettingsGroup
            title="Stored secrets"
            inset={false}
            footer="Write-only. Values are never shown on this device."
          >
            <SettingsRow
              label="Password"
              value={login.has_password ? "Stored" : "None"}
            />
            <SettingsRow
              label="Authenticator (TOTP)"
              value={login.has_totp ? "Stored" : "None"}
            />
          </SettingsGroup>
          <SettingsGroup title="Allowed origins" inset={false}>
            {login.allowed_origins.length ? (
              login.allowed_origins.map((origin) => (
                <SettingsRow key={origin} label={origin} mono selectable />
              ))
            ) : (
              <SettingsRow label="None" />
            )}
          </SettingsGroup>
          <SettingsGroup inset={false}>
            <SettingsRow
              label="Last used"
              value={formatMachineDate(login.last_used_at)}
            />
            <SettingsRow
              label="Updated"
              value={formatMachineDate(login.updated_at)}
            />
            <SettingsRow
              label="Created"
              value={formatMachineDate(login.created_at)}
            />
          </SettingsGroup>
          <SettingsGroup
            inset={false}
            footer="Edit this login's origins, username, password or authenticator on the web."
          >
            <SettingsRow
              centered
              destructive
              label={write.busy ? "Deleting…" : "Delete Saved Login"}
              accessibilityLabel={`Review deletion of ${login.label}`}
              disabled={blocked}
              onPress={() => review(login)}
            />
          </SettingsGroup>
        </>
      )}
    </WorkspacePage>
  );
}
