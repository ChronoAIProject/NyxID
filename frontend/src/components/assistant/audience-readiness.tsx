import type { AgentConfiguration } from "@/schemas/assistant-nyxagent";

/** Information only: owners can open access even while configuration is incomplete. */
export function AudienceReadiness({ configuration: c }: { configuration?: AgentConfiguration | null }) {
  return (
    <div className="space-y-1" role="status">
      {c ? <p>
        Instructions: {c.instructions_present ? "present" : "missing"}; persona: {c.persona_present ? "present" : "missing"}; memory notes: {c.memory_notes}; skills: {c.skills}; service grants: {c.service_grants}; account read: {c.account_read ? "yes" : "no"}.
      </p> : <p>Agent configuration has not been checked.</p>}
      <p>These facts do not confirm complete business information or a successful customer test. Opening access is your choice. During model setup, customer requests wait briefly or receive a safe setup failure.</p>
    </div>
  );
}
