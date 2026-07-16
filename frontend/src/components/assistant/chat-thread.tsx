import { useEffect, useRef } from "react";
import { AlertCircle, User } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ApprovalCard } from "@/components/assistant/blocks/approval-card";
import { ArtifactBlock } from "@/components/assistant/blocks/artifact-block";
import { ConnectCard } from "@/components/assistant/blocks/connect-card";
import { RunCard } from "@/components/assistant/blocks/run-card";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import type { AssistantMessage, ContentBlock } from "@/types/assistant";

function UnsupportedContent() {
  return (
    <div className="flex items-center gap-2 rounded-lg border border-border bg-overlay px-3 py-2 text-[11px] text-muted-foreground">
      <AlertCircle className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
      Unsupported assistant content
    </div>
  );
}

function blockId(block: unknown): string {
  if (
    typeof block === "object" &&
    block !== null &&
    "block_id" in block &&
    typeof block.block_id === "string"
  ) {
    return block.block_id;
  }
  return "unsupported-block";
}

function renderBlock(
  block: unknown,
  onDecideApproval: (blockId: string, approved: boolean) => Promise<void>,
) {
  if (typeof block !== "object" || block === null || !("type" in block)) {
    return <UnsupportedContent />;
  }
  const typed = block as ContentBlock;
  switch (typed.type) {
    case "text":
      return <TextBlock text={typed.text} />;
    case "connect_card":
      return <ConnectCard block={typed} />;
    case "run":
      return <RunCard block={typed} />;
    case "approval_card":
      return (
        <ApprovalCard
          block={typed}
          onDecide={(approved) => onDecideApproval(typed.block_id, approved)}
        />
      );
    case "artifact":
      return <ArtifactBlock block={typed} />;
    default:
      return <UnsupportedContent />;
  }
}

export function ChatThread({
  messages,
  onDecideApproval,
}: {
  readonly messages: readonly AssistantMessage[];
  readonly onDecideApproval: (
    blockId: string,
    approved: boolean,
  ) => Promise<void>;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const element = scrollRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [messages]);

  if (messages.length === 0) {
    return (
      <div className="flex flex-1 items-center justify-center px-6 text-center">
        <div>
          <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-xl border border-nyx-secondary-400/20 bg-nyx-secondary-400/[0.06]">
            <NyxidIcon className="h-5 w-5" />
          </div>
          <p className="mt-3 text-[13px] font-medium text-foreground">
            Start a new conversation
          </p>
          <p className="mt-1 text-[11px] text-muted-foreground">
            Ask NyxID to work with your connected services.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div
      ref={scrollRef}
      className="min-h-0 flex-1 overflow-y-auto overscroll-contain"
    >
      <div className="mx-auto flex w-full max-w-[680px] flex-col gap-6 px-4 py-6 sm:px-6 sm:py-8">
        {messages.map((message) => (
          <article key={message.id} className="flex items-start gap-3">
            <div
              className={`flex h-[26px] w-[26px] shrink-0 items-center justify-center rounded-lg border ${
                message.role === "assistant"
                  ? "border-nyx-secondary-400/20 bg-nyx-secondary-400/[0.06]"
                  : "border-hairline bg-overlay-strong"
              }`}
            >
              {message.role === "assistant" ? (
                <NyxidIcon className="h-[13px] w-[13px]" />
              ) : (
                <User className="h-[13px] w-[13px] text-muted-foreground" />
              )}
            </div>
            <div className="min-w-0 flex-1">
              <span className="sr-only">{message.role}</span>
              {message.schema_version !== 1 ? (
                <UnsupportedContent />
              ) : (
                <div className="space-y-3">
                  {(message.blocks as unknown[]).map((block, index) => (
                    <div key={`${blockId(block)}-${String(index)}`}>
                      {renderBlock(block, onDecideApproval)}
                    </div>
                  ))}
                </div>
              )}
            </div>
          </article>
        ))}
      </div>
    </div>
  );
}
