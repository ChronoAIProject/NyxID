import { useEffect, useRef, useState } from "react";
import { Alert } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { ContentScreenProps } from "../../app/navigationTypes";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { workspaceChatNavigationAction } from "../../app/workspaceChatNavigation";
import {
  ActionButton,
  Card,
  Field,
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
  AssistantComposerSettings,
  useComposerAccessMode,
} from "./AssistantComposerSettings";
import {
  COMPOSER_ACCESS_DESCRIPTION,
  COMPOSER_ACCESS_LABEL,
} from "./composerSettings";
import { AssistantWorkingContext } from "./AssistantWorkingContext";

export function AssistantDetailsScreen({
  navigation,
  route,
}: ContentScreenProps<"AssistantDetails">) {
  const runtime = useGroupAvailability();
  const client = useQueryClient();
  const { conversationId, agentId: requestedAgent } = route.params;
  const scope = `${runtime.epoch}:${conversationId ?? "new"}:${requestedAgent ?? "nyxbot"}`;
  const historyKey = ["assistant", "details", runtime.epoch, conversationId];
  const history = useQuery({
    queryKey: historyKey,
    queryFn: ({ signal }) =>
      assistantApi.history(conversationId!, undefined, signal),
    enabled: runtime.active && !!conversationId,
    retry: false,
    refetchInterval: runtime.active && conversationId ? 5000 : false,
  });
  const agents = useQuery({
    queryKey: ["workspace", "agents", "drawer", runtime.epoch],
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
  const access = useComposerAccessMode(agent?.access_mode_override);
  const [title, setTitle] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [deleted, setDeleted] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [permissionsOpen, setPermissionsOpen] = useState(false);
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
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    setTitle(undefined);
    setError(undefined);
    setNotice(undefined);
    setDeleted(false);
    setPermissionsOpen(false);
  }, [scope]);
  const dirty =
    !deleted && title !== undefined && title !== conversation?.title;
  useWorkspaceBack({
    active: false,
    defaultRole: "back",
    onBack: navigation.goBack,
    disabled: busy,
    discardOnClose: dirty
      ? {
          title: "Discard title changes?",
          message: "Your unsaved chat title will be lost.",
        }
      : undefined,
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
  async function mutate(remove: boolean) {
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
    setNotice(undefined);
    try {
      if (remove) await assistantApi.delete(conversationId);
      else {
        const result = await assistantApi.rename(
          conversationId,
          title ?? conversation!.title,
        );
        if (result.id !== conversationId)
          throw new Error(
            "The server returned a different chat. Please reload.",
          );
        if (!alive()) return;
        await client.cancelQueries({ queryKey: historyKey, exact: true });
        if (!alive()) return;
        client.setQueryData<typeof history.data>(historyKey, (page) =>
          page ? { ...page, conversation: result } : page,
        );
      }
      if (!alive()) return;
      await Promise.allSettled([
        client.invalidateQueries({ queryKey: ["workspace", "drawer-chats"] }),
        client.invalidateQueries({ queryKey: ["workspace", "agents"] }),
        ...(remove ? [] : [history.refetch()]),
      ]);
      if (!alive()) return;
      setTitle(undefined);
      if (remove) setDeleted(true);
      else setNotice("Chat title saved.");
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
  return (
    <WorkspacePage title="Chat details" showHeading={false}>
      {!runtime.online && (
        <Notice message="You’re offline. Reconnect to change this chat." />
      )}
      {conversationId && history.isPending && (
        <Label muted>Loading chat details…</Label>
      )}
      {(history.isError || agents.isError) && (
        <>
          <Notice tone="error" message="Couldn’t load chat details." />
          <ActionButton
            label="Retry details"
            variant="secondary"
            disabled={!runtime.active}
            onPress={() => {
              if (conversationId) void history.refetch();
              void agents.refetch();
            }}
          />
        </>
      )}
      {error && <Notice tone="error" message={error} />}
      {notice && <Notice message={notice} />}
      <ActionButton
        label="Chat history"
        variant="secondary"
        onPress={() => navigation.push("AssistantHistory", { agentId })}
      />
      {conversationId && (
        <ActionButton
          label="Working context"
          variant="secondary"
          onPress={() =>
            navigation.push("AssistantWorkingContext", { conversationId })
          }
        />
      )}
      <Card>
        <Label>Agent</Label>
        <Label>
          {agent?.display_name ||
            agent?.name ||
            conversation?.agent?.display_name ||
            conversation?.agent?.name ||
            (requestedAgent ? "Agent" : "NyxBot")}
        </Label>
        <Label muted>
          {agent?.owner_kind === "org"
            ? agent.owner_name || "Organization"
            : agent?.kind === "specialist"
              ? "Personal specialist"
              : "Your personal assistant"}
        </Label>
      </Card>
      <Card>
        <Label>Permissions</Label>
        <Label>
          {access ? COMPOSER_ACCESS_LABEL[access] : "Loading permissions…"}
        </Label>
        {access && <Label muted>{COMPOSER_ACCESS_DESCRIPTION[access]}</Label>}
        {agent?.access_mode_override && (
          <Label muted>This agent’s override is read-only here.</Label>
        )}
        <ActionButton
          label="Change permissions"
          variant="secondary"
          disabled={
            !runtime.active ||
            busy ||
            !agents.data ||
            !!agent?.access_mode_override ||
            !!conversation?.active_turn ||
            agent?.status === "destroyed" ||
            agent?.can_use === false
          }
          onPress={() => setPermissionsOpen(true)}
        />
        <AssistantComposerSettings
          hideTrigger
          scope={scope}
          agentOverride={agent?.access_mode_override}
          activeTurn={!!conversation?.active_turn}
          disabled={
            !runtime.active ||
            busy ||
            !agents.data ||
            agent?.status === "destroyed" ||
            agent?.can_use === false
          }
          visible={permissionsOpen}
          onOpenChange={setPermissionsOpen}
        />
      </Card>
      {agentId && (
        <ActionButton
          label="Agent details"
          variant="secondary"
          onPress={() => navigation.push("AgentDetail", { agentId })}
        />
      )}
      {conversationId && (
        <Card>
          <Field
            label="Chat title"
            value={title ?? conversation?.title ?? ""}
            onChangeText={setTitle}
            maxLength={200}
            autoCapitalize="sentences"
            editable={!unavailable}
          />
          <ActionButton
            label="Save title"
            loading={busy}
            disabled={unavailable || !dirty || !(title ?? "").trim()}
            onPress={() => void mutate(false)}
          />
          <ActionButton
            label="Delete chat"
            variant="danger"
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
                    onPress: () => void mutate(true),
                  },
                ],
              )
            }
          />
        </Card>
      )}
    </WorkspacePage>
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
