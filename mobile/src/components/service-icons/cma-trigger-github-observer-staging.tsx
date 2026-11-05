import { Eye } from "lucide-react-native";
import type { ServiceIconProps } from "./index";
import { CompositeBadgeWrapper } from "./_shared";
import { CmaGlyph } from "./cma";

export default function CmaTriggerGithubObserverStagingIcon({
  size = 20,
  color,
}: ServiceIconProps) {
  return (
    <CompositeBadgeWrapper
      size={size}
      color={color}
      badge={<Eye strokeWidth={2.5} size={size * 0.7 - 2} color={color} />}
    >
      <CmaGlyph
        data-slug="cma-trigger-github-observer-staging"
        size={size}
        color={color}
      />
    </CompositeBadgeWrapper>
  );
}
