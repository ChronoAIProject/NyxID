// Composite glyph for the Google Cloud catalog tile: Google G + Lucide
// `Cloud` badge in NyxID accent purple. The accent (second tone) lives on the
// Lucide badge wrapper; the primary brand glyph stays `currentColor` only.
import { Cloud } from "lucide-react";
import { GoogleGlyph } from "./_shared";

export default function ApiGoogleCloudIcon({
  className,
}: {
  className?: string;
}) {
  return (
    <span
      className={`relative inline-flex h-5 w-5 items-center justify-center ${
        className ?? ""
      }`}
    >
      <GoogleGlyph data-slug="api-google-cloud" className="h-5 w-5" />
      <Cloud
        aria-hidden="true"
        className="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 text-nyx-secondary-400 bg-card/95 rounded-sm p-px"
      />
    </span>
  );
}
