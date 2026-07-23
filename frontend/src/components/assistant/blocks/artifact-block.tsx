import { useState } from "react";
import { ChevronRight, Download, FileText } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { ArtifactContentBlock } from "@/types/assistant";

function formatSize(sizeBytes: number): string {
  if (sizeBytes < 1024) return `${String(sizeBytes)} B`;
  return `${(sizeBytes / 1024).toFixed(1)} KB`;
}

export function ArtifactBlock({
  block,
}: {
  readonly block: ArtifactContentBlock;
}) {
  const [open, setOpen] = useState(false);
  return (
    <section className="overflow-hidden rounded-xl border border-border/70 bg-card">
      <div className="flex min-w-0 items-center gap-3 px-3 py-2.5">
        <button
          type="button"
          onClick={() => setOpen((value) => !value)}
          aria-expanded={open}
          className="flex min-w-0 flex-1 items-center gap-3 text-left"
        >
          <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline bg-overlay-strong text-muted-foreground">
            <FileText className="h-4 w-4" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[11px] font-medium text-foreground">
              {block.name}
            </span>
            <span className="block font-mono text-[9px] text-text-tertiary">
              {block.mime} - {formatSize(block.size_bytes)}
            </span>
          </span>
          <ChevronRight
            className={`h-4 w-4 shrink-0 text-text-tertiary transition-transform ${open ? "rotate-90" : ""}`}
          />
        </button>
        {/* §3.5 gives the block a `download_url`; honour it when the producer
            sends one. Until then the control stays visible but inert, and says
            so rather than 404ing on an empty href. */}
        {block.download_url ? (
          <Button asChild variant="ghost" size="icon">
            <a
              href={block.download_url}
              download={block.name}
              target="_blank"
              rel="noopener noreferrer"
              aria-label={`Download ${block.name}`}
            >
              <Download />
            </a>
          </Button>
        ) : (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                disabled
                aria-label={`Download ${block.name}`}
              >
                <Download />
              </Button>
            </TooltipTrigger>
            <TooltipContent>
              This artifact carries no download URL yet
            </TooltipContent>
          </Tooltip>
        )}
      </div>
      {open && block.preview && (
        <pre className="whitespace-pre-wrap border-t border-border/50 bg-overlay px-4 py-3 font-mono text-[10px] leading-relaxed text-muted-foreground">
          {block.preview}
        </pre>
      )}
    </section>
  );
}
