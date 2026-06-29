import { useState } from "react";
import { Check, Copy, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn, copyToClipboard } from "@/lib/utils";
import { toast } from "sonner";

interface CopyableUrlCalloutProps {
  readonly label: string;
  readonly url: string;
  readonly description?: string;
  readonly docsHref?: string;
  readonly className?: string;
}

/**
 * Card-like callout that surfaces a URL the user must register somewhere
 * externally (webhook URL, OAuth redirect URI, etc.). Renders the URL in a
 * monospace `<code>` with a one-tap Copy button, an optional muted
 * description below it, and an optional "Learn more →" link to platform docs.
 *
 * Built to back the Wave B channel-bot webhook checklist and the migrated
 * `OAuthCallbackGuidance` shim — same visual recipe, fewer bespoke copies.
 */
export function CopyableUrlCallout({
  label,
  url,
  description,
  docsHref,
  className,
}: CopyableUrlCalloutProps) {
  const [copied, setCopied] = useState(false);

  async function handleCopy() {
    try {
      await copyToClipboard(url);
      setCopied(true);
      toast.success(`${label} copied`);
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy");
    }
  }

  return (
    <div
      className={cn(
        "space-y-2 rounded-xl border border-border bg-muted/40 p-3",
        className,
      )}
    >
      <p className="text-xs font-medium text-foreground">{label}</p>
      <div className="relative">
        <code className="flex min-h-[40px] items-center break-all rounded-lg border border-border bg-background px-3 py-2 pr-11 font-mono text-[12px] leading-relaxed text-foreground">
          {url}
        </code>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          className="absolute right-1.5 top-1.5 h-8 w-8 shrink-0"
          onClick={() => void handleCopy()}
          aria-label={`Copy ${label}`}
        >
          {copied ? (
            <Check className="h-3.5 w-3.5 text-success" />
          ) : (
            <Copy className="h-3.5 w-3.5" />
          )}
          <span className="sr-only">Copy {label}</span>
        </Button>
      </div>
      {description ? (
        <p className="text-xs text-muted-foreground">{description}</p>
      ) : null}
      {docsHref ? (
        <a
          href={docsHref}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-1 text-xs text-primary hover:underline"
        >
          Learn more →
          <ExternalLink className="h-3 w-3" aria-hidden="true" />
        </a>
      ) : null}
    </div>
  );
}
