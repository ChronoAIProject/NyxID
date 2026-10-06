import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  MachineChoices,
  MachineSetup,
  MachineUpdateStatus,
} from "@/schemas/machines";

export function useMachineSetup(id: string | undefined) {
  return useQuery({
    queryKey: ["machine-setup", id],
    queryFn: () => api.get<MachineSetup>(`/machines/setups/${id}`),
    enabled: Boolean(id),
    refetchInterval: (query) =>
      [
        "connected",
        "expired",
        "declined",
        "failed",
        "permissions_missing",
        "offline",
      ].includes(query.state.data?.status ?? "")
        ? false
        : 1000,
  });
}
export function useCreateMachineSetup() {
  return useMutation({
    mutationFn: (choices: MachineChoices) =>
      api.post<MachineSetup>("/machines/setups", choices),
  });
}
// Setup credentials are intentionally never put in the Query cache or persistence.
export async function issueMachineSetup(id: string, choices: MachineChoices) {
  return api.post<{ token: string }>(`/machines/setups/${id}/token`, choices);
}
export function useMachinePairPreview() {
  return useMutation({
    mutationFn: (code: string) =>
      api.post<MachineSetup>("/machines/pair/preview", { code }),
  });
}
export function useMachinePairDecision() {
  return useMutation({
    mutationFn: (body: {
      code: string;
      approve: boolean;
      automatic_updates?: boolean;
    }) => api.post<MachineSetup>("/machines/pair/decide", body),
  });
}
export function useMachineSettings(id: string) {
  const query = useQueryClient();
  return useMutation({
    mutationFn: (body: {
      machine_confirm: "none" | "changes" | "all";
      allow_single_user_saved_logins: boolean;
      acknowledge_single_user_risk: boolean;
    }) => api.put(`/nodes/${id}/machine-settings`, body),
    onSuccess: () => query.invalidateQueries({ queryKey: ["nodes"] }),
  });
}

export interface MachineDesktopMetadata {
  readonly display?: "secure" | "dev";
  readonly node_id: string;
  readonly session_id: string;
  readonly conversation_id: string | null;
  readonly status: string;
  readonly reason: string | null;
  readonly context_id?: string | null;
}
export function useMachineDesktops(conversation: string) {
  return useQuery({
    queryKey: ["machine-desktops", conversation],
    queryFn: () =>
      api.get<MachineDesktopMetadata[]>(
        `/assistant/nyxagent/machines?${new URLSearchParams({ conversation_id: conversation })}`,
      ),
    enabled: Boolean(conversation),
    refetchInterval: 5000,
  });
}

export function useMachineUpdates() {
  return useQuery({
    queryKey: ["machine-updates"],
    queryFn: () => api.get<MachineUpdateStatus[]>("/machines/updates"),
    refetchInterval: 3000,
  });
}
export function useStartMachineUpdate(id: string) {
  const query = useQueryClient();
  return useMutation({
    mutationFn: () =>
      api.post<{ status: string; reason?: string }>(
        `/machines/${id}/update`,
        {},
      ),
    onSuccess: () => query.invalidateQueries({ queryKey: ["machine-updates"] }),
  });
}
export function useMachineUpdatePolicy(id: string) {
  const query = useQueryClient();
  return useMutation({
    mutationFn: (automatic: boolean) =>
      api.put(`/machines/${id}/update-policy`, { automatic }),
    onSuccess: () => query.invalidateQueries({ queryKey: ["machine-updates"] }),
  });
}

export function useVerifiedUpdaterImage() {
  return useQuery({
    queryKey: ["machine-updater-image"],
    queryFn: () => api.get<{version: string; image: string | null; status: string}>("/machines/updater-image"),
    refetchInterval: (query) => query.state.data?.image ? 3_600_000 : 5000,
  });
}
