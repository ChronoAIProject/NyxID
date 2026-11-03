import {
  createContext,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
} from "react";

export interface GroupedFormEntry {
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
  readonly saving: boolean;
  readonly save: () => Promise<void>;
} | null>(null);

/** Each form retains its validation, payload and error handling; the scope only coordinates actions. */
export function useGroupedForm(entry: GroupedFormEntry) {
  const context = useContext(GroupedFormContext);
  const register = context?.register;
  const remove = context?.remove;
  const id = useId();
  const latest = useRef(entry);
  useLayoutEffect(() => {
    latest.current = entry;
  });
  const { label, dirty, pending, disabled } = entry;
  useEffect(() => {
    register?.(id, {
      label,
      dirty,
      pending,
      disabled,
      save: () => latest.current.save(),
      discard: () => latest.current.discard(),
    });
  }, [register, id, label, dirty, pending, disabled]);
  useEffect(() => () => remove?.(id), [remove, id]);
  return context !== null;
}
