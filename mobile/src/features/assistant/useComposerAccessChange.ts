import { useEffect, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { assistantApi } from "../../lib/api/assistantApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import type { ComposerAccessMode } from "./composerSettings";

export const COMPOSER_ACCESS_MODES = [
  "autopilot",
  "nyxmode",
  "ask_every_time",
] as const satisfies readonly ComposerAccessMode[];

/**
 * The owner permissions choice for a page row (Chat details). It shares the
 * composer's settings query and applies the same fences: one write at a time,
 * no write during a live turn, under an agent override, offline, after an
 * account change, or once the page has moved to another chat.
 */
export function useComposerAccessChange(props: {
  scope: string;
  agentOverride?: ComposerAccessMode | null;
  activeTurn: boolean;
  disabled: boolean;
}) {
  const availability = useGroupAvailability();
  const queryClient = useQueryClient();
  const epoch = getAuthEpoch();
  const settingsKey = ["assistant", "composer-settings", epoch];
  const settings = useQuery({
    queryKey: settingsKey,
    queryFn: ({ signal }) => assistantApi.settings(signal),
    enabled: availability.active,
    retry: false,
  });
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const lock = useRef(false);
  const mounted = useRef(true);
  const latest = useRef({ props, availability });
  latest.current = { props, availability };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    setError(undefined);
  }, [props.scope]);
  const mode = props.agentOverride ?? settings.data?.access_mode;
  const locked =
    props.disabled ||
    !availability.active ||
    props.activeTurn ||
    !!props.agentOverride ||
    pending ||
    !settings.data;
  async function change(value: ComposerAccessMode) {
    const state = latest.current;
    if (
      lock.current ||
      !state.availability.available() ||
      state.props.disabled ||
      state.props.activeTurn ||
      getAuthEpoch() !== epoch
    )
      return;
    if (state.props.agentOverride || !settings.data || value === mode) return;
    const scope = state.props.scope;
    const current = () =>
      mounted.current &&
      getAuthEpoch() === epoch &&
      latest.current.props.scope === scope;
    lock.current = true;
    setPending(true);
    setError(undefined);
    try {
      const result = await assistantApi.updateAccessMode(value);
      if (current()) queryClient.setQueryData(settingsKey, result);
    } catch (cause) {
      if (current())
        setError(
          cause instanceof Error
            ? cause.message
            : "Could not change permissions.",
        );
    } finally {
      lock.current = false;
      if (mounted.current) setPending(false);
    }
  }
  return {
    mode,
    locked,
    pending,
    error,
    loadFailed: settings.isError,
    retry: () => void settings.refetch(),
    change,
  };
}
