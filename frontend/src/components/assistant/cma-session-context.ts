import { createContext, useContext } from "react";
import type {
  CmaSessionReference,
  CmaSessionState,
} from "@/schemas/cma-session";

export interface Observation {
  state: CmaSessionState;
  reconnect: () => void;
}
export interface ObservedCmaSession extends Observation {
  session: CmaSessionReference;
}
export const CmaSessionContext = createContext<readonly ObservedCmaSession[]>(
  [],
);
export const cmaSessionKey = (session: CmaSessionReference) =>
  `${session.connection.id}:${session.agentId}`;
export function useObservedCmaSessions() {
  return useContext(CmaSessionContext);
}
