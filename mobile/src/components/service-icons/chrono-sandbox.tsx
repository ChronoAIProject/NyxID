import { Box } from "lucide-react-native";
import type { ServiceIconProps } from "./index";
import { ChronoServiceIcon } from "./chrono";

export default function ChronoSandboxIcon({
  size = 20,
  color,
}: ServiceIconProps) {
  return (
    <ChronoServiceIcon
      size={size}
      color={color}
      badge={<Box strokeWidth={2.5} size={size * 0.7 - 2} color={color} />}
      slug="chrono-sandbox"
    />
  );
}
