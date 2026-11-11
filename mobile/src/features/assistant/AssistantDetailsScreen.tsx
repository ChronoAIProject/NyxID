import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Alert } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { ContentScreenProps } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { workspaceChatNavigationAction } from "../../app/workspaceChatNavigation";
import { AgentAvatar } from "../../components/AgentAvatar";
import {
  SettingsGroup,
  SettingsMenuRow,
  SettingsPage,
  SettingsRow,
  SettingsTextField,
} from "../../components/settings/SettingsList";
import { SettingsHeaderButton } from "../../components/settings/SettingsHeaderButton";
import {
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { agentsApi } from "../../lib/api/agentsApi";
import { assistantApi } from "../../lib/api/assistantApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import { resolveChatAgentId } from "./assistantAgentContext";
import {
  COMPOSER_ACCESS_DESCRIPTION,
  COMPOSER_ACCESS_LABEL,
} from "./composerSettings";
import {
  COMPOSER_ACCESS_MODES,
  useComposerAccessChange,
} from "./useComposerAccessChange";
import { AssistantWorkingContext } from "./AssistantWorkingContext";

const detailsKey = (epoch: number, conversationId?: string) => [
  "assistant",
  "details",
  epoch,
  conversationId,
];
const agentsKey = (epoch: number) => ["workspace", "agents", "drawer", epoch];

/** The chat's history page and the agent list, shared by details and its edit page. */
function useChatDetailsData(conversationId?: string, requestedAgent?: string) {
  const runtime = useGroupAvailability();
  const history = useQuery({
    queryKey: detailsKey(runtime.epoch, conversationId),
    queryFn: ({ signal }) =>
      assistantApi.history(conversationId!, undefined, signal),
    enabled: runtime.active && !!conversationId,
    retry: false,
    refetchInterval: runtime.active && conversationId ? 5000 : false,
  });
  const agents = useQuery({
    queryKey: agentsKey(runtime.epoch),
    queryFn: ({ signal }) => agentsApi.list(signal),
    enabled: runtime.active,
    retry: false,
  });
  const conversation = history.data?.conversation;
  const agentId = resolveChatAgentId(
    agents.data?.agents,
    conversation?.agent?.id,
    requestedAgent,
  );
  const agent = agents.data?.agents.find((a) => a.id === agentId);
  return { runtime, history, agents, conversation, agentId, agent };
}

/**
 * Chat details as an iOS grouped list: the agent, the chat's own values
 * (title opens a focused edit page), permissions as a native menu, and
 * Delete Chat as the irreversible action at the bottom.
 */
export function AssistantDetailsScreen({
  navigation,
  route,
}: ContentScreenProps<"AssistantDetails">) {
  const { conversationId, agentId: requestedAgent } = route.params;
  const { runtime, history, agents, conversation, agentId, agent } =
    useChatDetailsData(conversationId, requestedAgent);
  const client = useQueryClient();
  const scope = `${runtime.epoch}:${conversationId ?? "new"}:${requestedAgent ?? "nyxbot"}`;
  const [busy, setBusy] = useState(false);
  const [deleted, setDeleted] = useState(false);
  const [error, setError] = useState<string>();
  const mounted = useRef(true);
  const lock = useRef(false);
  const unavailable =
    !runtime.active ||
    !conversation ||
    agent?.status === "destroyed" ||
    agent?.can_use === false ||
    busy ||
    deleted;
  const latest = useRef({ scope, runtime, unavailable });
  latest.current = { scope, runtime, unavailable };
  const access = useComposerAccessChange({
    scope,
    agentOverride: agent?.access_mode_override,
    activeTurn: !!conversation?.active_turn,
    disabled:
      !runtime.active ||
      busy ||
      !agents.data ||
      agent?.status === "destroyed" ||
      agent?.can_use === false,
  });
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    setError(undefined);
    setDeleted(false);
  }, [scope]);
  useWorkspaceBack({
    active: false,
    defaultRole: "back",
    onBack: navigation.goBack,
    disabled: busy,
  });
  const alive = () =>
    mounted.current &&
    latest.current.scope === scope &&
    runtime.epoch === getAuthEpoch();
  useEffect(() => {
    if (!deleted || busy) return;
    const action = workspaceChatNavigationAction(
      navigation.getParent()?.getState(),
      { kind: "agent", agentId },
    );
    if (action) navigation.dispatch(action);
  }, [deleted, busy, navigation, agentId]);
  async function remove() {
    if (
      lock.current ||
      !alive() ||
      !latest.current.runtime.available() ||
      latest.current.unavailable ||
      !conversationId
    )
      return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      await assistantApi.delete(conversationId);
      if (!alive()) return;
      await Promise.allSettled([
        client.invalidateQueries({ queryKey: ["workspace", "drawer-chats"] }),
        client.invalidateQueries({ queryKey: ["workspace", "agents"] }),
      ]);
      if (!alive()) return;
      setDeleted(true);
    } catch (cause) {
      if (alive())
        setError(
          cause instanceof Error
            ? cause.message
            : "Could not update the chat. Try again.",
        );
    } finally {
      lock.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  const agentName =
    agent?.display_name ||
    agent?.name ||
    conversation?.agent?.display_name ||
    conversation?.agent?.name ||
    (requestedAgent ? "Agent" : "NyxBot");
  const identity = agent ?? conversation?.agent;
  const permissionsFooter = [
    access.mode ? COMPOSER_ACCESS_DESCRIPTION[access.mode] : undefined,
    agent?.access_mode_override
      ? "This agent’s override is read-only here."
      : undefined,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <SettingsPage>
      {!runtime.online && (
        <Notice message="You’re offline. Reconnect to change this chat." />
      )}
      {conversationId && history.isPending && (
        <Label muted>Loading chat details…</Label>
      )}
      {(history.isError || agents.isError) && (
        <Notice tone="error" message="Couldn’t load chat details." />
      )}
      {(history.isError || agents.isError) && (
        <SettingsGroup inset={false}>
          <SettingsRow
            tone="action"
            label="Retry"
            accessibilityLabel="Retry details"
            disabled={!runtime.active}
            onPress={() => {
              if (conversationId) void history.refetch();
              void agents.refetch();
            }}
          />
        </SettingsGroup>
      )}
      {error && <Notice tone="error" message={error} />}
      {access.error && <Notice tone="error" message={access.error} />}
      <SettingsGroup>
        <SettingsRow
          leading={
            identity ? <AgentAvatar agent={identity} size="md" /> : undefined
          }
          label={agentName}
          detail={
            agent?.owner_kind === "org"
              ? agent.owner_name || "Organization"
              : agent?.kind === "specialist"
                ? "Personal specialist"
                : "Your personal assistant"
          }
          accessibilityHint={agentId ? "Opens agent details" : undefined}
          onPress={
            agentId
              ? () => navigation.push("AgentDetail", { agentId })
              : undefined
          }
        />
      </SettingsGroup>
      <SettingsGroup inset={false}>
        {conversationId && (
          <SettingsRow
            label="Title"
            value={conversation?.title}
            disabled={unavailable}
            accessibilityHint="Opens the chat title to edit"
            onPress={() =>
              navigation.push("AssistantTitleEdit", {
                conversationId,
                agentId: requestedAgent,
              })
            }
          />
        )}
        <SettingsRow
          label="Chat history"
          onPress={() => navigation.push("AssistantHistory", { agentId })}
        />
        {conversationId && (
          <SettingsRow
            label="Working context"
            onPress={() =>
              navigation.push("AssistantWorkingContext", { conversationId })
            }
          />
        )}
      </SettingsGroup>
      <SettingsGroup
        inset={false}
        footer={permissionsFooter || undefined}
      >
        {access.mode ? (
          <SettingsMenuRow
            label="Permissions"
            value={access.mode}
            options={COMPOSER_ACCESS_MODES.map((mode) => ({
              value: mode,
              label: COMPOSER_ACCESS_LABEL[mode],
              detail: COMPOSER_ACCESS_DESCRIPTION[mode],
            }))}
            disabled={access.locked}
            onChange={(mode) => void access.change(mode)}
          />
        ) : (
          <SettingsRow label="Permissions" value="Loading permissions…" />
        )}
        {access.loadFailed && (
          <SettingsRow
            tone="action"
            label="Retry loading permissions"
            disabled={!runtime.active}
            onPress={access.retry}
          />
        )}
      </SettingsGroup>
      {conversationId && (
        <SettingsGroup
          inset={false}
          footer="Deletes its saved messages and permissions. Active work will be stopped."
        >
          <SettingsRow
            centered
            destructive
            label={busy ? "Deleting…" : "Delete Chat"}
            disabled={unavailable}
            onPress={() =>
              Alert.alert(
                "Delete this chat?",
                "This deletes its saved messages and permissions. Active work will be stopped.",
                [
                  { text: "Cancel", style: "cancel" },
                  {
                    text: "Delete",
                    style: "destructive",
                    onPress: () => void remove(),
                  },
                ],
              )
            }
          />
        </SettingsGroup>
      )}
    </SettingsPage>
  );
}

/** Chat details › Title: one field, Save in the header, guarded like the old form. */
export function AssistantTitleEditScreen({
  navigation,
  route,
}: ContentScreenProps<"AssistantTitleEdit">) {
  const { conversationId, agentId: requestedAgent } = route.params;
  const { runtime, history, conversation, agent } = useChatDetailsData(
    conversationId,
    requestedAgent,
  );
  const client = useQueryClient();
  const historyKey = detailsKey(runtime.epoch, conversationId);
  const scope = `${runtime.epoch}:${conversationId}`;
  const [title, setTitle] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string>();
  const mounted = useRef(true);
  const lock = useRef(false);
  const unavailable =
    !runtime.active ||
    !conversation ||
    agent?.status === "destroyed" ||
    agent?.can_use === false ||
    busy ||
    saved;
  const latest = useRef({ scope, runtime, unavailable });
  latest.current = { scope, runtime, unavailable };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    setTitle(undefined);
    setError(undefined);
    setSaved(false);
  }, [scope]);
  const draft = title ?? conversation?.title ?? "";
  const dirty = !saved && title !== undefined && title !== conversation?.title;
  const canSave = !unavailable && dirty && !!draft.trim();
  const alive = () =>
    mounted.current &&
    latest.current.scope === scope &&
    runtime.epoch === getAuthEpoch();
  async function save() {
    if (
      lock.current ||
      !alive() ||
      !latest.current.runtime.available() ||
      latest.current.unavailable
    )
      return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      const result = await assistantApi.rename(
        conversationId,
        title ?? conversation!.title,
      );
      if (result.id !== conversationId)
        throw new Error("The server returned a different chat. Please reload.");
      if (!alive()) return;
      await client.cancelQueries({ queryKey: historyKey, exact: true });
      if (!alive()) return;
      client.setQueryData<typeof history.data>(historyKey, (page) =>
        page ? { ...page, conversation: result } : page,
      );
      await Promise.allSettled([
        client.invalidateQueries({ queryKey: ["workspace", "drawer-chats"] }),
        client.invalidateQueries({ queryKey: ["workspace", "agents"] }),
        history.refetch(),
      ]);
      if (!alive()) return;
      setTitle(undefined);
      setSaved(true);
    } catch (cause) {
      if (alive())
        setError(
          cause instanceof Error
            ? cause.message
            : "Could not update the chat. Try again.",
        );
    } finally {
      lock.current = false;
      if (mounted.current) setBusy(false);
    }
  }
  const saveRef = useRef(save);
  saveRef.current = save;
  const headerRight = useCallback(
    () => (
      <SettingsHeaderButton
        enabled={canSave}
        busy={busy}
        onPress={() => void saveRef.current()}
      />
    ),
    [canSave, busy],
  );
  const options = useMemo(() => ({ headerRight }), [headerRight]);
  useWorkspaceBack({
    active: false,
    defaultRole: "back",
    onBack: navigation.goBack,
    disabled: busy,
    options,
    discardOnClose: dirty
      ? {
          title: "Discard title changes?",
          message: "Your unsaved chat title will be lost.",
        }
      : undefined,
  });
  useEffect(() => {
    if (saved && !busy) navigation.goBack();
  }, [saved, busy, navigation]);
  return (
    <SettingsPage>
      {!runtime.online && (
        <Notice message="You’re offline. Reconnect to change this chat." />
      )}
      {history.isPending && <Label muted>Loading chat details…</Label>}
      {history.isError && (
        <Notice tone="error" message="Couldn’t load chat details." />
      )}
      {error && <Notice tone="error" message={error} />}
      <SettingsGroup
        inset={false}
        footer="Shown in your chat list and chat history."
      >
        <SettingsTextField
          accessibilityLabel="Chat title"
          value={draft}
          onChangeText={(text) => {
            setTitle(text);
            setError(undefined);
          }}
          placeholder="Chat title"
          maxLength={200}
          autoCapitalize="sentences"
          autoFocus
          editable={!unavailable}
          returnKeyType="done"
          onSubmitEditing={() => {
            if (canSave) void save();
          }}
        />
      </SettingsGroup>
    </SettingsPage>
  );
}

export function AssistantWorkingContextScreen({
  navigation,
  route,
}: ContentScreenProps<"AssistantWorkingContext">) {
  const runtime = useGroupAvailability();
  const [busy, setBusy] = useState(false);
  useWorkspaceBack({
    active: false,
    defaultRole: "back",
    onBack: navigation.goBack,
    disabled: busy,
  });
  return (
    <WorkspacePage title="Working context" showHeading={false}>
      <AssistantWorkingContext
        key={route.params.conversationId}
        conversationId={route.params.conversationId}
        active={runtime.active}
        onBusyChange={setBusy}
        onReset={() => {}}
        onManageMemory={(agentId) =>
          navigation.push("AgentDetail", { agentId })
        }
      />
    </WorkspacePage>
  );
}
