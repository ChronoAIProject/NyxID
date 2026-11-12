import { MessageSquare } from "lucide-react-native";
import {
  DiscordGlyph,
  LarkFamilyGlyph,
  SlackGlyph,
  TelegramGlyph,
  TwitterGlyph,
  WhatsappGlyph,
} from "./service-icons/_shared";

/** A bare platform mark for a channel badge, without catalog sub-badges. */
export function ChannelPlatformGlyph({
  platform,
  size,
  color,
}: {
  platform: string;
  size: number;
  color: string;
}) {
  const props = { size, color, testID: `channel-platform-${platform}` };
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
