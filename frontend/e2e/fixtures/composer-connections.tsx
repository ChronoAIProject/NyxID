import { useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { NyxAgentComposer } from "../../src/components/assistant/nyxagent-composer";
import { Button } from "../../src/components/ui/button";
import { useApplyTheme } from "../../src/hooks/use-theme";
import {
  client,
  restoreConnections,
  updateConnection,
} from "./composer-connections-state";
import { useAuthStore } from "../../src/stores/auth-store";
import type { User } from "../../src/types/api";
import "../../src/i18n";
import "../../src/app.css";

useAuthStore.setState({
  user: { id: "fixture" } as User,
  isAuthenticated: true,
  isLoading: false,
});

export function ComposerConnectionsFixture() {
  useApplyTheme("assistant");
  const [locked, setLocked] = useState(false);
  const [sent, setSent] = useState<string[]>([]);
  return (
    <main className="mx-auto max-w-2xl space-y-4 p-6">
      <h1>Composer connection availability fixture</h1>
      <p>
        Local fixture only; sends are recorded below without contacting a
        provider.
      </p>
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          onClick={() => updateConnection("work", { credential_missing: true })}
        >
          Invalidate work connection
        </Button>
        <Button
          type="button"
          onClick={restoreConnections}
        >
          Restore connections
        </Button>
      </div>
      <Button type="button" onClick={() => setLocked((previous) => !previous)}>
        Toggle composer lock
      </Button>
      <NyxAgentComposer
        ownerUserId="fixture"
        draftKey="availability"
        scope={{ kind: "conversations", id: "fixture" }}
        disabled={locked}
        active={false}
        sending={false}
        onStop={async () => {}}
        onSend={async (text) => {
          setSent((previous) => [...previous, text]);
        }}
      />
      <section className="space-y-2 rounded-xl border border-border p-4">
        <h2>Local send log · {sent.length} messages</h2>
        <p className="text-13 text-muted-foreground">
          Test messages only. Nothing is sent to a real provider.
        </p>
        <output
          aria-label="Submitted messages"
          className="block whitespace-pre-wrap break-words"
        >
          {sent.length ? sent.join("\n\n") : "No messages sent yet."}
        </output>
      </section>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <QueryClientProvider client={client}>
    <ComposerConnectionsFixture />
  </QueryClientProvider>,
);
