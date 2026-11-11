// AWS Cost Explorer catalog tile: AWS smile + Lucide `Calculator` badge. See
// `CompositeBadgeWrapper` in `_shared.tsx` for the visual layering rules.
import { Calculator } from "lucide-react-native";
import { AwsGlyph, CompositeBadgeWrapper } from "./_shared";

export default function AwsCostExplorerIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <CompositeBadgeWrapper
      size={size}
      color={color}
      badge={
        <Calculator strokeWidth={2.5} size={size * 0.7 - 2} color={color} />
      }
    >
      <AwsGlyph data-slug="aws-cost-explorer" size={size} color={color} />
    </CompositeBadgeWrapper>
  );
}
