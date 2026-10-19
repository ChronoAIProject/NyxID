import { createContext, useContext } from "react";

export const AssistantDrawerDismissContext = createContext<() => void>(
  () => undefined,
);

export function useAssistantDrawerDismiss() {
  return useContext(AssistantDrawerDismissContext);
}
