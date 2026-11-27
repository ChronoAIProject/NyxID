import { createContext, type Dispatch, type SetStateAction } from "react";
import type {
  AvatarConfig,
  AvatarMood,
} from "@/lib/assistant/agent-avatar-model";
export interface AvatarPreview {
  agentId: string;
  config: AvatarConfig;
  mood: AvatarMood;
  eventKey: string;
}
/** Only the details header consumes a draft; roster and chat retain saved identity. */
export const AvatarPreviewContext = createContext<Dispatch<
  SetStateAction<AvatarPreview | null>
> | null>(null);
