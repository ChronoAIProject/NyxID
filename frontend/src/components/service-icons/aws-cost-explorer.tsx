// Composite glyph for the AWS Cost Explorer catalog tile: AWS smile + Lucide
// `Calculator` badge in NyxID accent purple. The accent (second tone) lives on
// the Lucide badge wrapper; the primary brand glyph stays `currentColor`
// only.
import { Calculator } from "lucide-react";
import { AwsGlyph } from "./_shared";

export default function AwsCostExplorerIcon({
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
      <AwsGlyph data-slug="aws-cost-explorer" className="h-5 w-5" />
      <Calculator
        aria-hidden="true"
        className="absolute -bottom-0.5 -right-0.5 h-2.5 w-2.5 text-nyx-secondary-400 bg-card/95 rounded-sm p-px"
      />
    </span>
  );
}
