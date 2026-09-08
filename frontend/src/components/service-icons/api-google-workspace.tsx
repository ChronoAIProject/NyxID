import { BriefcaseBusiness } from "lucide-react";
import { CompositeBadgeWrapper, GoogleGlyph } from "./_shared";

export default function ApiGoogleWorkspaceIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<BriefcaseBusiness strokeWidth={2.5} />}
    >
      <GoogleGlyph data-slug="api-google-workspace" className="h-full w-full" />
    </CompositeBadgeWrapper>
  );
}
