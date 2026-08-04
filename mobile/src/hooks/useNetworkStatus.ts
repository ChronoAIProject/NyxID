import { useCallback, useEffect, useState } from "react";
import NetInfo from "@react-native-community/netinfo";
import { onlineManager } from "@tanstack/react-query";

export function useNetworkStatus() {
  const [isConnected, setIsConnected] = useState(true);

  useEffect(() => {
    const unsubscribe = NetInfo.addEventListener((state) => {
      setIsConnected(state.isConnected ?? true);
    });
    return () => unsubscribe();
  }, []);

  /** Re-probe connectivity, sync onlineManager, and return whether we're online. */
  const recheckConnection = useCallback(async () => {
    const state = await NetInfo.fetch();
    const connected = !!state.isConnected;
    setIsConnected(connected);
    if (connected) onlineManager.setOnline(true);
    return connected;
  }, []);

  return { isConnected, recheckConnection };
}
