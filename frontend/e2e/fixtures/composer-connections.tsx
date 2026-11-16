import { useState } from "react";
import { createRoot } from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import { NyxAgentComposer } from "../../src/components/assistant/nyxagent-composer";
import { Button } from "../../src/components/ui/button";
import { useApplyTheme } from "../../src/hooks/use-theme";
import {
  client,
  keys,
  queryKey,
  updateConnection,
} from "./composer-connections-state";
import "../../src/app.css";

export function ComposerConnectionsFixture() {
  useApplyTheme("assistant");
  const [sent, setSent] = useState<string[]>([]);
  return (
    <main className="mx-auto max-w-2xl space-y-4 p-6">
      <h1>Composer connection availability fixture</h1>
      <p>
        Local fixture only; sends are recorded below without contacting a
        provider.
      </p>
      <Button
        type="button"
        onClick={() => updateConnection("work", { credential_missing: true })}
      >
        Invalidate work connection
      </Button>
      <Button type="button" onClick={() => client.setQueryData(queryKey, keys)}>
        Restore connections
      </Button>
      <NyxAgentComposer
        ownerUserId="fixture"
        draftKey="availability"
        scope={{ kind: "conversations", id: "fixture" }}
        active={false}
        sending={false}
        onStop={async () => {}}
        onSend={async (text) => {
          setSent((previous) => [...previous, text]);
        }}
      />
      <output aria-label="Submitted messages">{JSON.stringify(sent)}</output>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(
  <QueryClientProvider client={client}>
    <ComposerConnectionsFixture />
  </QueryClientProvider>,
);
