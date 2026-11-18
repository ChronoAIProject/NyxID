import { createContext, useContext, useEffect, useId, useLayoutEffect, useRef } from "react";
import type { GroupedFormEntry } from "./grouped-form";

export interface GroupedDraft extends Pick<GroupedFormEntry, "dirty" | "pending" | "discard"> {
  readonly owner: string;
}

export const GroupedDraftOwnerContext = createContext("");
export const GroupedDraftContext = createContext<{
  readonly register: (id: string, entry: GroupedDraft) => void;
  readonly remove: (id: string) => void;
  readonly confirmDiscard: (owner?: string) => Promise<boolean>;
  readonly entries: ReadonlyMap<string, GroupedDraft>;
  readonly pending: boolean;
  readonly finish: (discard: boolean) => void;
} | null>(null);

export function useGroupedDraftGuard() {
  return useContext(GroupedDraftContext);
}

/** Registers a draft for dismissal without changing the form's save UI. */
export function useGroupedDraft(entry: Pick<GroupedFormEntry, "dirty" | "pending" | "discard">) {
  const context = useGroupedDraftGuard();
  const owner = useContext(GroupedDraftOwnerContext);
  const register = context?.register;
  const remove = context?.remove;
  const id = useId();
  const latest = useRef(entry);
  const { dirty, pending } = entry;
  useLayoutEffect(() => { latest.current = entry; });
  useLayoutEffect(() => {
    register?.(id, { owner, dirty, pending, discard: () => latest.current.discard() });
  }, [register, id, owner, dirty, pending]);
  useEffect(() => () => remove?.(id), [remove, id]);
}
