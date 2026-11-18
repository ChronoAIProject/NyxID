import type { ServiceIconProps } from "./index";
import { SupabaseGlyph } from "./_shared";

export default function ApiSupabaseIcon({
  size = 20,
  color,
}: ServiceIconProps) {
  return <SupabaseGlyph data-slug="api-supabase" size={size} color={color} />;
}
