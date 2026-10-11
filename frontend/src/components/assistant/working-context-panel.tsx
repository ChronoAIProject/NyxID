import { useState } from "react";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { assistantContextSchema, type AssistantContext } from "@/schemas/assistant-context";

export function WorkingContextPanel({ conversationId, onManageMemory }: { readonly conversationId: string; readonly onManageMemory?: () => void }) {
  const [open, setOpen] = useState(false);
  const [context, setContext] = useState<AssistantContext>();
  async function load() {
    const response = await assistantHttp(`/assistant/nyxagent/conversations/${conversationId}/context`);
    setContext(assistantContextSchema.parse(await response.json())); setOpen(true);
  }
  async function reset() {
    const response = await assistantHttp(`/assistant/nyxagent/conversations/${conversationId}/context/reset`, { method: "POST" });
    setContext(assistantContextSchema.parse(await response.json()));
  }
  return <div className="rounded border border-hairline px-3 py-2 text-11">
    <button type="button" aria-expanded={open} className="underline" onClick={() => void load()}>Working context</button>
    {open ? <div className="mt-2 space-y-1 text-text-tertiary"><div>Goal: {context?.goal ?? "No goal recorded"}</div><div>Binding: {context?.binding_state ?? "unknown"}</div><div>Completed: {context?.completed_actions.join(", ") || "None"}</div><div>Pending: {context?.pending_actions.join(", ") || "None"}</div><div>Failed: {context?.failed_actions.join(", ") || "None"}</div><div>Last successful turn: {context?.last_successful_turn ?? "None"}</div><div>Reset reason: {context?.reset_reason ?? "None"}</div><button type="button" className="underline" onClick={() => void reset()}>Reset context</button> <button type="button" className="underline" onClick={onManageMemory}>Manage memory notes</button></div> : null}
  </div>;
}
