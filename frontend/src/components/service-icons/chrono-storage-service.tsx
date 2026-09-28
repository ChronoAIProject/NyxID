import { Database } from "lucide-react";
import type { ServiceIconProps } from "./index";
import { ChronoServiceIcon } from "./chrono";

export default function ChronoStorageServiceIcon({
  className,
}: ServiceIconProps) {
  return (
    <ChronoServiceIcon
      className={className}
      badge={<Database strokeWidth={2.5} />}
      slug="chrono-storage-service"
    />
  );
}
