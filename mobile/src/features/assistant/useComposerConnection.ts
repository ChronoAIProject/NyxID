import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { servicesApi } from "../../lib/api/servicesApi";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useGroupAvailability } from "../groups/useGroupRuntime";
import type { ServiceConnection } from "../services/serviceContracts";
import {
  composerConnectionAvailable,
  CONNECTION_UNAVAILABLE,
} from "./composerConnection";

export function useComposerConnection(scope: string, locked: boolean) {
  const availability = useGroupAvailability();
  const epoch = getAuthEpoch();
  const [selection, setSelection] = useState<{
    scope: string;
    epoch: number;
    key: ServiceConnection;
  }>();
  const [picker, setPicker] = useState<{ scope: string; epoch: number }>();
  useEffect(() => {
    setSelection(undefined);
    setPicker(undefined);
  }, [scope, epoch]);
  const query = useQuery({
    queryKey: ["assistant", "composer-connections", epoch],
    queryFn: ({ signal }) => servicesApi.pluginConnections(signal),
    enabled: availability.active,
    retry: false,
    staleTime: 0,
  });
  const connections = query.isError
    ? []
    : (query.data ?? []).filter(composerConnectionAvailable);
  const connection =
    selection?.scope === scope && selection.epoch === epoch
      ? selection.key
      : undefined;
  const unavailable =
    !!connection && !connections.some((key) => key.id === connection.id);
  const latest = useRef({
    scope,
    epoch,
    locked,
    availability,
    connection,
    unavailable,
    connections,
  });
  latest.current = {
    scope,
    epoch,
    locked,
    availability,
    connection,
    unavailable,
    connections,
  };
  const canChange = () =>
    latest.current.scope === scope &&
    latest.current.epoch === epoch &&
    !latest.current.locked &&
    epoch === getAuthEpoch() &&
    latest.current.availability.available();
  return {
    connection,
    unavailable,
    connections,
    open:
      picker?.scope === scope &&
      picker.epoch === epoch &&
      !locked &&
      availability.active,
    loading: query.isPending,
    error: query.isError,
    locked: locked || !availability.active,
    retry: () => {
      if (canChange()) void query.refetch();
    },
    choose: () => {
      if (canChange())
        setPicker({ scope: latest.current.scope, epoch: latest.current.epoch });
    },
    close: () => setPicker(undefined),
    select: (id: string) => {
      if (!canChange()) return;
      const key = latest.current.connections.find((key) => key.id === id);
      if (!key) return;
      setSelection({
        scope: latest.current.scope,
        epoch: latest.current.epoch,
        key,
      });
      setPicker(undefined);
    },
    remove: () => {
      if (canChange()) setSelection(undefined);
    },
    prefix: (text: string) => {
      const state = latest.current;
      if (
        state.scope !== scope ||
        state.epoch !== epoch ||
        epoch !== getAuthEpoch() ||
        !state.availability.available() ||
        state.unavailable
      )
        throw new Error(CONNECTION_UNAVAILABLE);
      return state.connection
        ? `/service ${state.connection.id}\n${text}`
        : text;
    },
    sent: () => {
      // An accepted/uncertain send consumes only its original message selection.
      const selected = connection;
      setSelection((value) =>
        value?.scope === scope &&
        value.epoch === epoch &&
        value.key.id === selected?.id
          ? undefined
          : value,
      );
    },
  };
}
export type ComposerConnectionState = ReturnType<typeof useComposerConnection>;
