import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp, appLocale } from "../../i18n/appTranslation";
import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Card,
  Label,
  ActionButton,
} from "../../components/workspace/WorkspaceUI";
import { assistantApi } from "../../lib/api/assistantApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { contextResetCopy } from "./assistantTurnPresentation";

export function AssistantWorkingContext({
  conversationId,
  active,
  onReset,
  onManageMemory,
  onBusyChange,
}: {
  conversationId: string;
  active: boolean;
  onReset: () => void;
  onManageMemory: (agentId: string) => void;
  onBusyChange?: (busy: boolean) => void;
}) {
  useAppLanguage();
  const epoch = getAuthEpoch();
  const mounted = useRef(true);
  const allowed = useRef(active);
  allowed.current = active;
  const lock = useRef(false);
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    onBusyChange?.(pending);
    return () => onBusyChange?.(false);
  }, [pending, onBusyChange]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const query = useQuery({
    queryKey: ["assistant", "context", epoch, conversationId],
    queryFn: ({ signal }) => assistantApi.context(conversationId, signal),
    enabled: active,
    retry: false,
    refetchInterval: (q) =>
      active && q.state.data?.active_turn_id ? 3000 : false,
  });
  const context = query.data;
  const checkpoint = context?.checkpoint;
  const labels = (actions?: { label: string }[]) =>
    actions?.length ? actions.map((a) => a.label).join(", ") : translateApp("None");
  async function reset() {
    if (
      !allowed.current ||
      lock.current ||
      context?.active_turn_id ||
      getAuthEpoch() !== epoch
    )
      return;
    lock.current = true;
    setPending(true);
    setFailed(false);
    try {
      await assistantApi.resetContext(conversationId);
      if (mounted.current && allowed.current && getAuthEpoch() === epoch) {
        setConfirming(false);
        await query.refetch();
        if (mounted.current && allowed.current && getAuthEpoch() === epoch)
          onReset();
      }
    } catch {
      if (mounted.current && getAuthEpoch() === epoch) setFailed(true);
    } finally {
      lock.current = false;
      if (mounted.current) setPending(false);
    }
  }
  return (
    <>
      {query.isPending ? (
        <Label muted>{translateApp("Loading working context…")}</Label>
      ) : null}
      {query.isError ? (
        <>
          <Label accessibilityRole="alert">
            {translateApp("Unable to load the working context.")}
          </Label>
          <ActionButton
            label={translateApp("Try again")}
            variant="secondary"
            disabled={!active}
            onPress={() => void query.refetch()}
          />
        </>
      ) : null}
      {context ? (
        <Card>
          <Label>{translateApp("Goal: {{goal}}", { goal: checkpoint?.goal ?? translateApp("No goal recorded") })}</Label>
          <Label muted>{(checkpoint
              ? (checkpoint.status === "completed" ? translateApp("Last checkpoint: completed · {{time}}", { time: new Date(checkpoint.updated_at).toLocaleString(appLocale())}) : translateApp("Last checkpoint: failed ({{code}}) · {{time}}", { code: checkpoint.error_code ?? "unknown", time: new Date(checkpoint.updated_at).toLocaleString(appLocale()) })) : translateApp("Last checkpoint: None yet"))}
          </Label>
          <Label muted>{(checkpoint?.last_successful_at
              ? translateApp("Last successful checkpoint: {{time}}", { time: new Date(checkpoint.last_successful_at).toLocaleString(
                  appLocale(),
                ) })
              : translateApp("Last successful checkpoint: None"))}
          </Label>
          <Label muted>{translateApp("Completed since then: {{labels}}", { labels: labels(checkpoint?.completed_actions) })}
          </Label>
          <Label muted>{translateApp("Failed or interrupted: {{labels}}", { labels: labels(checkpoint?.failed_actions) })}</Label>
          <Label muted>{translateApp("Running now: {{actions}}", { actions: context.pending_actions.join(", ") || translateApp("None") })}
          </Label>
          <Label muted>{(context.binding_state === "bound"
              ? translateApp("Live context: kept") : translateApp("Live context: starts fresh next turn"))}
          </Label>
          {context.reset_reason ? (
            <Label muted>{translateApp("Last reset: {{contextResetCopy}}", { contextResetCopy: contextResetCopy(context.reset_reason) })}
            </Label>
          ) : null}
          {failed ? (
            <Label accessibilityRole="alert">
              {context.active_turn_id
                ? translateApp(
                    "A turn is running. Stop it or wait, then reset.",
                  )
                : translateApp(
                    "Unable to reset the working context. Try again.",
                  )}
            </Label>
          ) : null}
          {confirming ? (
            <Card>
              <Label>
                {translateApp(
                  "Reset clears the live context and the checkpoint. The transcript stays; the next turn starts without a recap.",
                )}
              </Label>
              <ActionButton
                label={translateApp("Confirm reset")}
                disabled={!active || !!context.active_turn_id || pending}
                loading={pending}
                onPress={() => void reset()}
              />
              <ActionButton
                label={translateApp("Cancel")}
                variant="secondary"
                onPress={() => setConfirming(false)}
              />
            </Card>
          ) : (
            <ActionButton
              label={translateApp("Reset context")}
              variant="secondary"
              disabled={!active || !!context.active_turn_id}
              onPress={() => setConfirming(true)}
            />
          )}
          {context.agent_id ? (
            <ActionButton
              label={translateApp("Manage memory notes")}
              variant="secondary"
              disabled={!active || pending}
              onPress={() => onManageMemory(context.agent_id!)}
            />
          ) : null}
        </Card>
      ) : null}
    </>
  );
}
