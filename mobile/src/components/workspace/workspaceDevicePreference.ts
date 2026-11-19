import { useCallback, useEffect, useRef, useState } from "react";
import AsyncStorage from "@react-native-async-storage/async-storage";

/** A per-device on/off layout choice ("1"/"0"); unreadable storage keeps the default (off). */
export async function loadStoredFlag(key: string): Promise<boolean> {
  try {
    return (await AsyncStorage.getItem(key)) === "1";
  } catch {
    return false;
  }
}

export async function saveStoredFlag(key: string, on: boolean) {
  try {
    await AsyncStorage.setItem(key, on ? "1" : "0");
  } catch {
    // A failed write keeps this session's choice; the next launch uses the default.
  }
}

/**
 * Off until the saved value loads; a choice made before then wins. `loaded`
 * tells callers the value now reflects storage, so the restored state can
 * apply without animating.
 */
export function useStoredFlag(key: string) {
  const [state, setState] = useState({ on: false, loaded: false });
  const chosen = useRef(false);
  useEffect(() => {
    let alive = true;
    void loadStoredFlag(key).then((saved) => {
      if (alive)
        setState((current) =>
          chosen.current ? { ...current, loaded: true } : { on: saved, loaded: true },
        );
    });
    return () => {
      alive = false;
    };
  }, [key]);
  const set = useCallback(
    (on: boolean) => {
      chosen.current = true;
      setState({ on, loaded: true });
      void saveStoredFlag(key, on);
    },
    [key],
  );
  return [state.on, set, state.loaded] as const;
}
