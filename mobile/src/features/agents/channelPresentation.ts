import type { AssistantAgent } from "./agentContracts";

export type ChannelDetail =
  | { kind: "platform"; value: string }
  | { kind: "message"; value: string };

export function channelDetail(channel: AssistantAgent["channels"][number]): ChannelDetail {
  return channel.platform === "discord" && channel.gateway_native_activation === "gateway_native_draining"
    ? { kind: "message", value: "Discord intake is paused while the retained gateway release and safety drain are checked. NyxID keeps checking; retrying activation cannot bypass this safety check." }
    : { kind: "platform", value: channel.platform };
}
