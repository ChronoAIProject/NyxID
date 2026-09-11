import { Instagram } from "lucide-react";
import { AUTH_PROVIDER_ICONS } from "@/components/auth/provider-icons";
import { ServiceIcon } from "@/components/service-icon";
import { TelegramGlyph } from "@/components/service-icons/_shared";

export function BrandIcon({ brand }: { readonly brand: string }) {
  if (brand === "google" || brand === "github" || brand === "apple")
    return (
      <span
        className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-overlay-strong"
        aria-hidden="true"
      >
        {AUTH_PROVIDER_ICONS[brand]}
      </span>
    );
  if (brand === "telegram")
    return (
      <span className="nb-brand-icon nb-telegram">
        <TelegramGlyph />
      </span>
    );
  if (brand === "whatsapp")
    return (
      <span className="nb-brand-icon nb-whatsapp">
        <span className="nb-whatsapp-glyph" aria-hidden="true" />
      </span>
    );
  if (brand === "instagram")
    return (
      <span className="nb-brand-icon nb-instagram">
        <Instagram />
      </span>
    );
  if (brand === "notion")
    return (
      <span className="nb-brand-icon nb-notion" aria-hidden="true">
        N
      </span>
    );
  return (
    <ServiceIcon
      slug="api-facebook"
      size="sm"
      className={`nb-brand-icon nb-${brand}`}
    />
  );
}
