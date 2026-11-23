import { createContext } from "react";

/** Desktop destination for the selected agent's threads. Narrow screens keep them in the drawer. */
export const AssistantThreadsContext = createContext<HTMLElement | null>(null);
