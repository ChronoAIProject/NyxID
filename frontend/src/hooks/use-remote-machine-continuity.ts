import { useEffect, useSyncExternalStore } from "react";
import { useAuthStore } from "@/stores/auth-store";
import type { RemoteMachine } from "@/schemas/machines";
import {
  REMOTE_MACHINE_ACCEPTED_EVENT,
  observeAcceptedRemoteMachineIntent,
  parseAcceptedRemoteMachineIntent,
  remoteMachineAcceptedSnapshot,
} from "@/lib/remote-machine-intent";

function subscribe(listener: () => void) {
  window.addEventListener(REMOTE_MACHINE_ACCEPTED_EVENT, listener);
  window.addEventListener("storage", listener);
  return () => {
    window.removeEventListener(REMOTE_MACHINE_ACCEPTED_EVENT, listener);
    window.removeEventListener("storage", listener);
  };
}

/** Select the accepted response across list/detail lifetimes and reloads. */
export function useRemoteMachineContinuity(machineId: string, machine?: RemoteMachine) {
  const owner = useAuthStore((state) => state.user?.id);
  const raw = useSyncExternalStore(subscribe, () => {
    if (!owner) return null;
    try { return remoteMachineAcceptedSnapshot(owner, machineId); }
    catch { return "unreadable"; }
  }, () => null);
  let accepted;
  let readError: string | undefined;
  try { accepted = parseAcceptedRemoteMachineIntent(raw); }
  catch { readError = "The accepted request reference could not be read. Check status before starting another action."; }
  const observation = accepted?.currentObservation ??
    (accepted?.observedRevision !== undefined
      ? { operationId: accepted.operationId, revision: accepted.observedRevision }
      : undefined);
  const matchingMachine = machine?.node_id === machineId ? machine : undefined;
  const newerMachine = matchingMachine &&
    matchingMachine.revision > (observation?.revision ?? accepted?.body.expected_revision ?? -1);
  const current = newerMachine
    ? { operationId: matchingMachine.active_operation_id, revision: matchingMachine.revision }
    : observation;
  const operationId = current ? current.operationId : accepted?.operationId ?? matchingMachine?.active_operation_id;
  const machineReadBehind = Boolean(current && matchingMachine && current.revision > matchingMachine.revision);
  useEffect(() => {
    if (readError || !owner || useAuthStore.getState().user?.id !== owner ||
      machine?.node_id !== machineId) return;
    observeAcceptedRemoteMachineIntent(owner, machineId, machine.active_operation_id, machine.revision);
  }, [owner, machineId, machine?.node_id, machine?.active_operation_id, machine?.revision, raw, readError]);
  return {
    operationId: readError ? undefined : operationId ?? undefined,
    acceptedAction: accepted && operationId === accepted.operationId ? accepted.action : null,
    machineReadBehind,
    readError,
  };
}
