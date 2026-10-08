import { createContext } from "react";

// Account-panel descendants also include dialogs opened without a Radix trigger.
export const DialogFocusReturnContext = createContext(false);
