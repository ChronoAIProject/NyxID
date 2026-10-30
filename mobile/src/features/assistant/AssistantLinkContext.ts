import { createContext } from "react";

export const AssistantLinkContext = createContext<
  ((href: string, trigger?: number) => boolean) | null
>(null);
