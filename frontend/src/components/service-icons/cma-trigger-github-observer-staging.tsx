import { Eye } from "lucide-react";
import type { ServiceIconProps } from "./index";
import { FlatBadgeWrapper } from "./_shared";
import { CmaGlyph } from "./cma";

export default function CmaTriggerGithubObserverStagingIcon({
  className,
}: ServiceIconProps) {
  return (
    <FlatBadgeWrapper className={className} badge={<Eye strokeWidth={2.5} />}>
      <CmaGlyph
        data-slug="cma-trigger-github-observer-staging"
        className="h-full w-full"
      />
    </FlatBadgeWrapper>
  );
}
