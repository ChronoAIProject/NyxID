import { createContext, useContext } from "react";

export const CrmPollingContext = createContext(true);

export function useCrmQueryPolicy() {
  const polling = useContext(CrmPollingContext);
  return {
    refetchInterval: polling ? 30_000 : false,
    refetchOnMount: polling ? true : "always",
  } as const;
}
