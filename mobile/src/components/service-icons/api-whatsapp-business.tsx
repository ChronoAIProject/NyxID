import { BriefcaseBusiness } from "lucide-react-native";
import type { ServiceIconProps } from "./index";
import { CompositeBadgeWrapper, WhatsappGlyph } from "./_shared";

export default function ApiWhatsappBusinessIcon({
  size = 20,
  color,
}: ServiceIconProps) {
  return (
    <CompositeBadgeWrapper
      size={size}
      color={color}
      badge={
        <BriefcaseBusiness
          strokeWidth={2.5}
          size={size * 0.7 - 2}
          color={color}
        />
      }
    >
      <WhatsappGlyph
        data-slug="api-whatsapp-business"
        size={size}
        color={color}
      />
    </CompositeBadgeWrapper>
  );
}
