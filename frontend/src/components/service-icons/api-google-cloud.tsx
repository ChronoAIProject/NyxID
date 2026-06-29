// Google Cloud catalog tile: Google G + Lucide `Cloud` badge. The Cloud
// badge differentiates this tile from the plain `api-google` (OAuth/Google
// account) and `llm-google-ai` (Google AI Studio) tiles, which share the same
// "G" brand mark but use different badge glyphs.
import { Cloud } from "lucide-react";
import { CompositeBadgeWrapper, GoogleGlyph } from "./_shared";

export default function ApiGoogleCloudIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Cloud className="h-2.5 w-2.5" strokeWidth={2.5} />}
    >
      <GoogleGlyph data-slug="api-google-cloud" className="h-full w-full" />
    </CompositeBadgeWrapper>
  );
}
