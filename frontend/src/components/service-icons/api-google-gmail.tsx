import { Mail } from "lucide-react";
import { CompositeBadgeWrapper, GoogleGlyph } from "./_shared";

export default function ApiGoogleGmailIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Mail strokeWidth={2.5} />}
    >
      <GoogleGlyph data-slug="api-google-gmail" className="h-full w-full" />
    </CompositeBadgeWrapper>
  );
}
