import type { ServiceIconProps } from "./index";
import { SupabaseGlyph } from "./_shared";

export default function ApiSupabaseManagementIcon({
  className,
}: ServiceIconProps) {
  return (
    <SupabaseGlyph data-slug="api-supabase-management" className={className} />
  );
}
