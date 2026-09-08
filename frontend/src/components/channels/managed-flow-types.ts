import type { ManagedBootstrap } from "@/schemas/channel-managed";
import type {
  ChannelBotDetail,
  ChannelPlatform,
  CreateChannelBotResponse,
} from "@/types/channels";

export interface ManagedFlowProps {
  readonly platform: ChannelPlatform;
  readonly bootstrap: ManagedBootstrap;
  readonly label: string;
  readonly orgId: string | null;
  readonly onConnected: (bot: CreateChannelBotResponse) => void;
  readonly botId?: string;
}

export interface ManagedDetailProps {
  readonly bot: ChannelBotDetail;
  readonly orgId: string | null;
}
