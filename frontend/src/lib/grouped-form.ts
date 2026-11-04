import {
  createContext,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
} from "react";
import type { FormChange } from "./grouped-form-changes";
import { GroupedDraftContext, GroupedDraftOwnerContext } from "./grouped-drafts";

/** Async mutations can resolve before React commits their reset/unregistration. */
export const afterGroupedFormCommit = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

export interface GroupedFormEntry {
  readonly changes?: () => readonly FormChange[];
  readonly id?: string;
  readonly isDirty?: () => boolean;
  readonly label: string;
  readonly dirty: boolean;
  readonly pending?: boolean;
  readonly disabled?: boolean;
  readonly save: () => Promise<void>;
  readonly discard: () => void;
}

export const GroupedFormContext = createContext<{
  readonly register: (id: string, entry: GroupedFormEntry) => void;
  readonly remove: (id: string) => void;
  readonly entries: ReadonlyMap<string, GroupedFormEntry>;
  readonly viewport?: boolean;
  readonly requestReview: () => void;
  readonly saving: boolean;
  readonly save: () => Promise<readonly string[]>;
} | null>(null);

/** Each form retains its validation, payload and error handling; the scope only coordinates actions. */
export function useGroupedForm(entry: GroupedFormEntry) {
  const context = useContext(GroupedFormContext);
  const register = context?.register;
  const remove = context?.remove;
  const id = useId();
  const guard = useContext(GroupedDraftContext);
  const owner = useContext(GroupedDraftOwnerContext);
  const registerDraft = guard?.register;
  const removeDraft = guard?.remove;
  const latest = useRef(entry);
  useLayoutEffect(() => {
    latest.current = entry;
  });
  const { label, dirty, pending, disabled } = entry;
  useLayoutEffect(() => {
    const current = {
      id,
      isDirty: () => latest.current.dirty,
      label,
      changes: () => latest.current.changes?.() ?? [],
      dirty,
      pending,
      disabled,
      save: () => latest.current.save(),
      discard: () => latest.current.discard(),
    };
    register?.(id, current);
    registerDraft?.(id, { ...current, owner });
  }, [register, registerDraft, owner, id, label, dirty, pending, disabled]);
  useEffect(() => () => { remove?.(id); removeDraft?.(id); }, [remove, removeDraft, id]);
  return context !== null;
}
