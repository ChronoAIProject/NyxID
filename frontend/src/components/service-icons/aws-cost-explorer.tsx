// AWS Cost Explorer catalog tile: AWS smile + Lucide `Calculator` badge. See
// `CompositeBadgeWrapper` in `_shared.tsx` for the visual layering rules.
import { Calculator } from "lucide-react";
import { AwsGlyph, CompositeBadgeWrapper } from "./_shared";

export default function AwsCostExplorerIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <CompositeBadgeWrapper
      className={className}
      badge={<Calculator className="h-3.5 w-3.5" strokeWidth={2.5} />}
    >
      <AwsGlyph data-slug="aws-cost-explorer" className="h-5 w-5" />
    </CompositeBadgeWrapper>
  );
}
