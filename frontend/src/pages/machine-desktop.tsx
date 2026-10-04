import { useParams, useSearch } from "@tanstack/react-router";
import { MachineDesktopPanel } from "@/components/assistant/machine-desktop-panel";
export function MachineDesktopPage() {
  const { nodeId } = useParams({ strict: false }) as { nodeId: string };
  const { conversation_id, display } = useSearch({ strict: false }) as {
    conversation_id?: string;
    display?: "secure" | "dev";
  };
  return (
    <main className="min-h-dvh bg-background p-4">
      <MachineDesktopPanel
        nodeId={nodeId}
        conversationId={conversation_id}
        initialDisplay={display}
      />
    </main>
  );
}
