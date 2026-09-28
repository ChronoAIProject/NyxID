import { BriefcaseBusiness } from "lucide-react";
import type { ServiceIconProps } from "./index";
import { CompositeBadgeWrapper, WhatsappGlyph } from "./_shared";

export default function ApiWhatsappBusinessIcon({
  className,
}: ServiceIconProps) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<BriefcaseBusiness strokeWidth={2.5} />}
    >
      <WhatsappGlyph
        data-slug="api-whatsapp-business"
        className="h-full w-full"
      />
    </CompositeBadgeWrapper>
  );
}
