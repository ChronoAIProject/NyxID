import { createContext } from "react";
import type { NyxBotHandoff } from "@/schemas/nyxbot-handoff";
import type { PlatformSetupCompletion } from "./platform-setup";

export const NyxBotHandoffContext = createContext<
  | {
      ownerUserId?: string;
      sourceConversationId?: string;
      eligible?: boolean;
      continue: (
        handoff: NyxBotHandoff,
        onAdopt: (id: string) => void,
        receiptKey: string,
      ) => Promise<void>;
      open: (id: string) => void;
      complete?: (result: PlatformSetupCompletion) => Promise<void>;
      navigate?: (href: string) => void;
    }
  | undefined
>(undefined);
