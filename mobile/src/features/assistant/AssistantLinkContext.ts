import { createContext } from "react";

export const AssistantLinkContext = createContext<
  ((href: string, trigger?: number, bareAutolink?: boolean) => boolean) | null
>(null);
