import { useParams } from "@tanstack/react-router";
import { ChannelConnectLinkContent } from "@/components/channels/channel-connect-link-content";

export function ChannelConnectLinkPage() {
  const { token } = useParams({ strict: false }) as { token: string };
  return <ChannelConnectLinkContent token={token} />;
}
