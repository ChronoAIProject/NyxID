import type { ServiceIconProps } from "./index";
import ApiPosthogIcon from "./api-posthog";

export default function ApiPosthogEuIcon({
  size = 20,
  color,
}: ServiceIconProps) {
  return <ApiPosthogIcon slug="api-posthog-eu" size={size} color={color} />;
}
