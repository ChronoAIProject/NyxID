import { Box } from "lucide-react";
import type { ServiceIconProps } from "./index";
import { ChronoServiceIcon } from "./chrono";

export default function ChronoSandboxIcon({ className }: ServiceIconProps) {
  return (
    <ChronoServiceIcon
      className={className}
      badge={<Box strokeWidth={2.5} />}
      slug="chrono-sandbox"
    />
  );
}
