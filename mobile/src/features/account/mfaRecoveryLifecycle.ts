type Confirmation = { generation: number; authEpoch: number };

export type MfaRecoverySnapshot = {
  codes: readonly string[];
  count: number;
  confirming: boolean;
};

/** One-time output belongs to one mounted screen and authentication epoch.
 * Backgrounding masks it; only leaving that screen or acknowledging it
 * discards it. Nothing is persisted or put into the query/mutation cache. */
export function createMfaRecoveryLifecycle(getAuthEpoch: () => number) {
  const owner = getAuthEpoch();
  let generation = 0;
  let focused = false;
  let active = false;
  let pending: Confirmation | null = null;
  let codes: readonly string[] = [];

  const owned = () => {
    if (owner !== getAuthEpoch()) {
      clear();
      return false;
    }
    return focused;
  };
  const current = (confirmation: Confirmation) =>
    owned() &&
    pending === confirmation &&
    confirmation.generation === generation &&
    confirmation.authEpoch === owner;

  function clear() {
    generation += 1;
    pending = null;
    codes = [];
  }

  return {
    setFocused(value: boolean) {
      focused = value;
      if (!value) clear();
    },
    setActive(value: boolean) {
      active = value;
    },
    clear,
    beginConfirmation(): Confirmation | null {
      if (!owned() || !active || pending) return null;
      pending = { generation, authEpoch: owner };
      return pending;
    },
    completeConfirmation(
      confirmation: Confirmation,
      result: readonly string[],
    ) {
      if (!current(confirmation)) return false;
      codes = [...result];
      pending = null;
      return true;
    },
    finishConfirmation(confirmation: Confirmation) {
      if (!current(confirmation)) return false;
      pending = null;
      return true;
    },
    snapshot(): MfaRecoverySnapshot {
      if (!owned()) return { codes: [], count: 0, confirming: false };
      return {
        codes: active ? codes : [],
        count: codes.length,
        confirming: pending !== null,
      };
    },
  };
}
