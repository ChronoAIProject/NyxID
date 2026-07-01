// Feishu catalog tile: outline LarkFamilyGlyph. Feishu is the
// China-market brand of the same underlying product as Lark; the
// outline treatment distinguishes it from Lark's solid variant while
// preserving the shared bird silhouette.
import { LarkFamilyGlyph } from "./_shared";

export default function ApiFeishuIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <LarkFamilyGlyph
      variant="outline"
      data-slug="api-feishu"
      className={className}
    />
  );
}
