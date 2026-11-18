import type { ServiceIconProps } from "./index";
import { SupabaseGlyph } from "./_shared";

export default function ApiSupabaseManagementIcon({
  size = 20,
  color,
}: ServiceIconProps) {
  return (
    <SupabaseGlyph
      data-slug="api-supabase-management"
      size={size}
      color={color}
    />
  );
}
