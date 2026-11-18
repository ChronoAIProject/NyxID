// Lark catalog tile: solid LarkFamilyGlyph. Lark and Feishu share the
// same bird silhouette; the solid-vs-outline treatment is how the two
// regional variants stay visually distinct while preserving the shared
// brand mark. Lark = solid (international/primary brand); Feishu =
// outline. Bot variants pair the same solid/outline treatment with the
// standard Lucide Bot badge.
import { LarkFamilyGlyph } from "./_shared";

export default function ApiLarkIcon({
  size = 20,
  color,
}: {
  size?: number;
  color?: string;
}) {
  return (
    <LarkFamilyGlyph
      variant="solid"
      data-slug="api-lark"
      size={size}
      color={color}
    />
  );
}
