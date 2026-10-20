import type { ServiceIconProps } from "./index";
import { SupabaseGlyph } from "./_shared";

export default function ApiSupabaseIcon({ className }: ServiceIconProps) {
  return <SupabaseGlyph data-slug="api-supabase" className={className} />;
}
