import { useState } from "react";

export function WorkingContextPanel({ conversationId }: { readonly conversationId: string }) {
  const [open, setOpen] = useState(false);
  const [context, setContext] = useState<{ goal?: string; completed_actions?: string[]; pending_actions?: string[]; reset_reason?: string | null }>();
  async function load() {
    const response = await fetch(`/api/v1/assistant/nyxagent/conversations/${conversationId}/context`);
    if (response.ok) setContext(await response.json());
    setOpen(true);
  }
  async function reset() {
    await fetch(`/api/v1/assistant/nyxagent/conversations/${conversationId}/context/reset`, { method: "POST" });
    await load();
  }
  return <div className="rounded border border-hairline px-3 py-2 text-11">
    <button type="button" className="underline" onClick={() => void load()}>Working context</button>
    {open ? <div className="mt-2 space-y-1 text-text-tertiary"><div>Goal: {context?.goal ?? "No goal recorded"}</div><div>Completed: {context?.completed_actions?.join(", ") || "None"}</div><div>Pending: {context?.pending_actions?.join(", ") || "None"}</div><div>Reset reason: {context?.reset_reason ?? "None"}</div><button type="button" className="underline" onClick={() => void reset()}>Reset context</button> <a className="underline" href="/assistant/settings?tab=memory">Manage memory notes</a></div> : null}
  </div>;
}
