import { MessageSquare } from "lucide-react";
import {
  DiscordGlyph,
  LarkFamilyGlyph,
  SlackGlyph,
  TelegramGlyph,
  TwitterGlyph,
  WhatsappGlyph,
} from "@/components/service-icons/_shared";

/** A bare platform mark for a channel badge, without catalog sub-badges. */
export function ChannelPlatformGlyph({
  platform,
  className,
}: {
  readonly platform: string;
  readonly className?: string;
}) {
  const props = {
    className,
    "aria-hidden": true as const,
    "data-channel-platform": platform,
  };
  switch (platform) {
    case "telegram":
      return <TelegramGlyph {...props} />;
    case "lark":
      return <LarkFamilyGlyph {...props} variant="solid" />;
    case "feishu":
      return <LarkFamilyGlyph {...props} variant="outline" />;
    case "discord":
      return <DiscordGlyph {...props} />;
    case "slack":
      return <SlackGlyph {...props} />;
    case "whatsapp":
      return <WhatsappGlyph {...props} />;
    case "x":
    case "twitter":
      return <TwitterGlyph {...props} />;
    default:
      return <MessageSquare {...props} />;
  }
}
