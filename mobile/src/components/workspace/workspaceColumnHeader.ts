import { createContext } from "react";
import type { AgentIdentity } from "../agentAvatarModel";

type ColumnControl = {
  label: string;
  onPress: () => void;
  disabled?: boolean;
};

/**
 * A chat bar's controls for the Duo's full screen, where the bar is gone and
 * they sit in one grouped capsule in the status column below the restore
 * capsule: Menu (or Back), the agent (opening the identity pill's picker)
 * and More. The focused chat registers them (`useWorkspaceColumnHeader`);
 * the rail renders them.
 */
export type WorkspaceColumnHeaderControls = {
  leading: ColumnControl & {
    role: "menu" | "back";
    /** Lets the column's Menu/Back take the bar's place for focus restoration. */
    registerFocus?: (target: () => number | null, active: boolean) => void;
  };
  identity: ColumnControl & {
    agent?: AgentIdentity;
    members?: readonly AgentIdentity[];
  };
  more: ColumnControl;
};

/** Registers (or, with null, withdraws) the focused chat's column controls. */
export type WorkspaceColumnHeaderRegistry = (
  owner: symbol,
  controls: WorkspaceColumnHeaderControls | null,
) => void;

export const WorkspaceColumnHeaderContext =
  createContext<WorkspaceColumnHeaderRegistry>(() => {});
