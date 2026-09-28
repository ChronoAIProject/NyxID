import { Bot } from "lucide-react";
import type { ServiceIconProps } from "./index";
import { ChronoServiceIcon } from "./chrono";

export default function ChronoLlmPublicIcon({ className }: ServiceIconProps) {
  return (
    <ChronoServiceIcon
      className={className}
      badge={<Bot strokeWidth={2.5} />}
      slug="chrono-llm-public"
    />
  );
}
