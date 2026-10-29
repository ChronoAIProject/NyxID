import { create } from "zustand";
import { useAuthStore } from "@/stores/auth-store";

export interface MachineSettingsDraft {
  confirm?: "none" | "changes" | "all";
  allow?: boolean;
  automatic?: boolean;
  members: Record<string, boolean>;
  confirmed: Record<string, boolean>;
  saving?: boolean;
  errors?: string[];
}
const empty: MachineSettingsDraft = { members: {}, confirmed: {} };
/** Memory only. Never store setup tokens, credentials or per-agent authority here. */
export const useMachineSettingsDraftStore = create<{
  drafts: Record<string, MachineSettingsDraft>;
}>(() => ({ drafts: {} }));
useAuthStore.subscribe((next, previous) => {
  if (next.user?.id !== previous.user?.id)
    useMachineSettingsDraftStore.setState({ drafts: {} });
});
export function machineDraftKey(
  actor: string | undefined,
  owner: string,
  machine: string,
) {
  return JSON.stringify([actor, owner, machine]);
}
export function updateMachineDraft(
  key: string,
  actor: string | undefined,
  update: (draft: MachineSettingsDraft) => MachineSettingsDraft,
) {
  if (useAuthStore.getState().user?.id !== actor) return;
  useMachineSettingsDraftStore.setState((state) => ({
    drafts: { ...state.drafts, [key]: update(state.drafts[key] ?? empty) },
  }));
}
export function useMachineSettingsDraft(owner: string, machine: string) {
  const actor = useAuthStore((state) => state.user?.id);
  const key = machineDraftKey(actor, owner, machine);
  const draft = useMachineSettingsDraftStore(
    (state) => state.drafts[key] ?? empty,
  );
  return {
    draft,
    update: (update: (draft: MachineSettingsDraft) => MachineSettingsDraft) =>
      updateMachineDraft(key, actor, update),
    actor,
  };
}
