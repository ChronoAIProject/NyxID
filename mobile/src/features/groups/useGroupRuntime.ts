import { useCallback, useEffect, useRef, useState } from "react";
import { AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { groupsApi } from "../../lib/api/groupsApi";
import { ApiError } from "../../lib/api/ApiError";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { mergeGroupPage, mergeGroupMessages } from "./groupState";
import { restoreRefusedAssistantSend } from "../assistant/assistantTurnRequest";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import { groupKeys } from "./groupLiveRefresh";
import { useGroupsLive } from "./useGroupsLive";
import {
  groupPostRequestSchema,
  type GroupMessagesPage,
  type GroupPostRequest,
  type GroupPosted,
} from "./groupContracts";

export { groupKeys } from "./groupLiveRefresh";
export const groupError = (cause: unknown) =>
  cause instanceof Error
    ? cause.message
    : "Could not complete this action. Try again.";

/** Every group read and local callback belongs to one signed-in foreground view. */
export function useGroupAvailability() {
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const epoch = getAuthEpoch();
  const current = useRef({ epoch, focused, foreground, online: isConnected });
  current.current = { epoch, focused, foreground, online: isConnected };
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      current.current.foreground = state === "active";
      setForeground(state === "active");
    });
    return () => subscription.remove();
  }, []);
  const available = useCallback(
    () =>
      current.current.epoch === getAuthEpoch() &&
      current.current.focused &&
      current.current.foreground &&
      AppState.currentState === "active" &&
      current.current.online,
    [],
  );
  return {
    epoch,
    focused,
    foreground,
    online: isConnected,
    active: focused && foreground && isConnected,
    available,
  };
}

export function useGroupChat(groupId: string) {
  const availability = useGroupAvailability();
  const { epoch, active, available } = availability;
  const live = useGroupsLive(epoch, active, available);
  const client = useQueryClient();
  const key = [...groupKeys(epoch), "messages", groupId] as const;
  const scope = `${epoch}:${groupId}`;
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const mounted = useRef(true);
  const fastUntil = useRef(0);
  const lock = useRef(false);
  const olderLock = useRef(false);
  const recovery = useRef(false);
  const [sending, setSending] = useState(false);
  const [recoveryRequired, setRecoveryRequired] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [error, setError] = useState<string>();
  const alive = () =>
    mounted.current &&
    currentScope.current === scope &&
    epoch === getAuthEpoch();
  const query = useQuery({
    queryKey: key,
    queryFn: async ({ signal }) => {
      const page = await groupsApi.messages(groupId, undefined, signal);
      return mergeGroupPage(client.getQueryData<GroupMessagesPage>(key), page);
    },
    enabled: active,
    retry: false,
    refetchInterval: (state) =>
      livePollInterval(
        active
          ? state.state.data?.group.working_agent_ids.length ||
            Date.now() < fastUntil.current
            ? 1500
            : 15_000
          : false,
        live,
      ),
  });
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (active) void client.invalidateQueries({ queryKey: key, exact: true });
    else void client.cancelQueries({ queryKey: key, exact: true });
    // The scope deliberately includes the account epoch and exact group.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, scope, client]);

  async function loadOlder() {
    const before = client.getQueryData<GroupMessagesPage>(key)?.before_seq;
    if (!available() || !before || olderLock.current || lock.current) return;
    olderLock.current = true;
    setLoadingOlder(true);
    setError(undefined);
    try {
      const page = await groupsApi.messages(groupId, before);
      if (alive() && available())
        client.setQueryData<GroupMessagesPage>(key, (latest) =>
          mergeGroupPage(latest, page, "older"),
        );
    } catch (cause) {
      if (alive() && available()) setError(groupError(cause));
    } finally {
      olderLock.current = false;
      if (alive()) setLoadingOlder(false);
    }
  }

  async function perform<T>(action: () => Promise<T>): Promise<T | undefined> {
    if (!available() || lock.current || olderLock.current) return undefined;
    lock.current = true;
    setSending(true);
    setError(undefined);
    try {
      return await action();
    } catch (cause) {
      if (alive() && available()) setError(groupError(cause));
      throw cause;
    } finally {
      lock.current = false;
      if (alive()) setSending(false);
    }
  }

  async function post(body: GroupPostRequest): Promise<boolean> {
    // Refuse local mistakes before detaching uploads or dispatching the POST.
    const payload = groupPostRequestSchema.parse(body);
    if (recovery.current) return false;
    return (
      (await perform(async () => {
        let posted: GroupPosted;
        try {
          posted = await groupsApi.post(groupId, payload);
        } catch (cause) {
          const refused =
            cause instanceof ApiError &&
            restoreRefusedAssistantSend(cause, false);
          if (alive()) {
            if (!refused) {
              recovery.current = true;
              setRecoveryRequired(true);
            }
            setError(
              refused
                ? groupError(cause)
                : "Connection interrupted. This message may have been delivered. Refresh the crew and check its history before sending again.",
            );
          }
          // Unknown delivery consumes uploads: the server may have claimed them.
          // Only an explicit pre-admission 4xx keeps the original draft editable.
          return !refused;
        }
        if (!alive()) return true;
        fastUntil.current = Date.now() + 5000;
        client.setQueryData<GroupMessagesPage>(key, (latest) =>
          latest
            ? {
                ...latest,
                messages: mergeGroupMessages(latest.messages, [posted.message]),
                group: {
                  ...latest.group,
                  working_agent_ids: [
                    ...new Set([
                      ...latest.group.working_agent_ids,
                      ...posted.addressed_agent_ids,
                    ]),
                  ],
                  default_recipient_ids: posted.addressed_agent_ids.length
                    ? posted.addressed_agent_ids
                    : latest.group.default_recipient_ids,
                },
              }
            : latest,
        );
        void client.invalidateQueries({ queryKey: groupKeys(epoch) });
        void client.invalidateQueries({ queryKey: ["workspace", "agents"] });
        return true;
      })) === true
    );
  }
  async function recoverDelivery() {
    if (!available() || lock.current || olderLock.current) return;
    // Start after the uncertain POST has settled, cancelling any older poll.
    const refreshed = await query.refetch({ cancelRefetch: true });
    if (refreshed.isSuccess && alive() && available()) {
      recovery.current = false;
      setRecoveryRequired(false);
      setError(undefined);
    }
  }
  return {
    ...availability,
    ...query,
    sending,
    recoveryRequired,
    loadingOlder,
    actionError: error,
    post,
    recoverDelivery,
    perform,
    loadOlder,
  };
}
