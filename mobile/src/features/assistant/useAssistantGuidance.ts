import { useEffect, useRef, useState } from "react";
import { randomUUID } from "expo-crypto";
import { useQuery } from "@tanstack/react-query";
import { assistantApi } from "../../lib/api/assistantApi";
import { isApiError } from "../../lib/api/ApiError";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import type { AssistantConversation } from "./assistantTypes";

type Input = {
  scope: string;
  conversationId?: string;
  turn?: AssistantConversation["active_turn"];
  activeTurn: boolean;
  disabled: boolean;
  onRefresh: () => Promise<unknown>;
};
type Notice = { code: string | null; outcome?: string; text?: string };
export function useAssistantGuidance(input: Input) {
  const scope = input.scope;
  const availability = useGroupAvailability();
  const epoch = getAuthEpoch();
  const capabilities = useQuery({
    queryKey: ["assistant", "steer-capabilities", epoch, input.conversationId],
    queryFn: ({ signal }) =>
      assistantApi.capabilities(input.conversationId!, signal),
    enabled: !!input.conversationId && availability.active,
    staleTime: 60_000,
    retry: false,
  });
  const [noticeState, setNotice] = useState<{
    scope: string;
    epoch: number;
    value: Notice;
  }>();
  const [submitting, setSubmitting] = useState(false);
  const request = useRef<
    { text: string; turnId: string; id: string } | undefined
  >(undefined);
  const busy = useRef(false);
  const mounted = useRef(true);
  const latest = useRef({ input, epoch, availability, capabilities });
  latest.current = { input, epoch, availability, capabilities };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    request.current = undefined;
    setNotice(undefined);
  }, [input.scope, epoch]);
  const current = () =>
    mounted.current &&
    epoch === getAuthEpoch() &&
    latest.current.input.scope === scope &&
    latest.current.epoch === epoch;
  const canSteer = !!(
    input.activeTurn &&
    input.conversationId &&
    capabilities.data?.steer &&
    input.turn?.steering_allowed &&
    !input.turn.stop_requested
  );
  const notice =
    noticeState?.scope === input.scope && noticeState.epoch === epoch
      ? noticeState.value
      : undefined;
  const publish = (value: Notice) => {
    if (current()) setNotice({ scope: input.scope, epoch, value });
  };
  const enabled = () =>
    current() &&
    !latest.current.input.disabled &&
    latest.current.availability.available();
  async function submit(
    text: string,
    attachmentCount: number,
  ): Promise<boolean> {
    const state = latest.current;
    const turn = state.input.turn;
    if (
      !enabled() ||
      busy.current ||
      !state.input.conversationId ||
      !state.input.activeTurn ||
      !turn?.steering_allowed ||
      turn.stop_requested ||
      !state.capabilities.data?.steer
    )
      return false;
    if (
      !text.trim() ||
      attachmentCount ||
      [...text].length > state.capabilities.data.steer.max_chars
    ) {
      publish({ code: "invalid_request" });
      return false;
    }
    if (
      request.current?.text !== text ||
      request.current.turnId !== turn.turn_id
    )
      request.current = { text, turnId: turn.turn_id, id: randomUUID() };
    const identity = request.current;
    busy.current = true;
    setSubmitting(true);
    if (current()) setNotice(undefined);
    try {
      const result = await assistantApi.steer(
        state.input.conversationId,
        text,
        turn.turn_id,
        identity.id,
      );
      publish({ code: result.code, outcome: result.outcome });
      if (current() && request.current === identity)
        request.current = undefined;
      return true;
    } catch (cause) {
      const code = isApiError(cause) ? cause.errorKey : "steer_unavailable";
      publish({ code, text });
      if (code === "no_active_turn") {
        if (current() && request.current === identity)
          request.current = undefined;
        return true;
      }
      // An uncertain write is consumed, never automatically re-sent. If the
      // person explicitly retries the same text/turn, reuse its exact UUID.
      if (
        code === "steer_unavailable" ||
        !isApiError(cause) ||
        cause.statusCode >= 500
      )
        return true;
      if (current() && code !== "starting" && request.current === identity)
        request.current = undefined;
      return false;
    } finally {
      if (enabled())
        await latest.current.input.onRefresh().catch(() => undefined);
      busy.current = false;
      if (mounted.current) setSubmitting(false);
    }
  }
  async function sendAsMessage(send: (text: string) => Promise<boolean>) {
    if (
      !enabled() ||
      busy.current ||
      latest.current.input.activeTurn ||
      notice?.code !== "no_active_turn" ||
      !notice.text
    )
      return;
    busy.current = true;
    setSubmitting(true);
    try {
      if (await send(notice.text)) {
        if (current()) setNotice(undefined);
      }
    } catch {
      /* Ordinary send owns its error; keep this explicit recovery action. */
    } finally {
      busy.current = false;
      if (mounted.current) setSubmitting(false);
    }
  }
  return {
    canSteer,
    submitting,
    notice,
    maxChars: capabilities.data?.steer?.max_chars ?? 32_000,
    submit,
    sendAsMessage,
  };
}
