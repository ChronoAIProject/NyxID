// Pure brand glyph for the Twilio catalog tile.
// Two-tone rule: primary glyph uses `currentColor` only (no accent here).
import { TwilioGlyph } from "./_shared";

export default function ApiTwilioIcon({
  className,
}: {
  className?: string;
}) {
  return <TwilioGlyph data-slug="api-twilio" className={className} />;
}
