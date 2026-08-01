import { useMemo } from "react";
import { FileJson2 } from "lucide-react";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import type { AssistantWireLogExchange } from "@/schemas/assistant-wire-log";
import { replayWireExchange } from "@/lib/assistant/wire-replay";

function blockLabel(type: string): string {
  switch (type) {
    case "run":
      return "Run ledger";
    case "connect_card":
      return "Connection card";
    case "action_card":
      return "Action card";
    case "approval_card":
      return "Approval card";
    case "artifact":
      return "Media / artifact";
    default:
      return type;
  }
}

export function AssistantWireReplayView({
  exchange,
}: {
  readonly exchange: AssistantWireLogExchange;
}) {
  const projection = useMemo(() => replayWireExchange(exchange), [exchange]);
  if (!projection) return null;

  return (
    <div className="space-y-3" data-testid="wire-replay-view">
      {projection.partial ? (
        <p className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-[11px] text-warning">
          Partial replay: the delivered capture was truncated or ended without a
          clean terminal frame.
        </p>
      ) : null}
      <p className="text-[11px] leading-5 text-text-tertiary">
        Text uses the production chat renderer. Cards and media are not
        replayed; their original source frames are shown as inert placeholders.
      </p>
      {projection.state.messages.length === 0 ? (
        <p className="py-5 text-center text-[11px] text-text-tertiary">
          No renderable chat content in this capture.
        </p>
      ) : (
        <div className="space-y-3">
          {projection.state.messages.map((message) => (
            <div key={message.id} className="space-y-2">
              {message.blocks.map((block) => {
                if (block.type === "text") {
                  return (
                    <div key={block.block_id} className="pl-[7px]">
                      <TextBlock text={block.text} />
                    </div>
                  );
                }
                const frames =
                  projection.sourceFramesByBlockId[block.block_id] ?? [];
                return (
                  <section
                    key={block.block_id}
                    aria-label={`${blockLabel(block.type)} not replayed`}
                    className="overflow-hidden rounded-lg border border-border/70 bg-background/50"
                  >
                    <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2">
                      <FileJson2 className="h-3.5 w-3.5 text-text-tertiary" />
                      <span className="text-[11px] font-medium text-foreground">
                        {blockLabel(block.type)}
                      </span>
                      <span className="ml-auto text-[10px] text-text-tertiary">
                        Not replayed
                      </span>
                    </div>
                    <p className="px-3 pt-2 text-[10px] font-medium uppercase text-text-tertiary">
                      Original source frame JSON
                    </p>
                    <pre className="max-h-56 overflow-auto whitespace-pre-wrap break-all p-3 pt-1 font-mono text-[10px] leading-5 text-muted-foreground">
                      {JSON.stringify(
                        frames.length === 1 ? frames[0] : frames,
                        null,
                        2,
                      )}
                    </pre>
                  </section>
                );
              })}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
