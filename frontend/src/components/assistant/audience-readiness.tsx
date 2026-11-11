import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import type { AgentConfiguration } from "@/schemas/assistant-nyxagent";

/** Information only: owners can open access even while configuration is incomplete. */
export function AudienceReadiness({ configuration: c }: { configuration?: AgentConfiguration | null }) {
  useUiTranslation();
  const presence = (present: boolean) => translateUi(present ? "present" : "missing");
  return (
    <div className="space-y-1" role="status">
      {c ? <p>
        {translateUi("Instructions: {{instructions}}; persona: {{persona}}; memory notes: {{memoryNotes}}; skills: {{skills}}; service grants: {{serviceGrants}}; account read: {{accountRead}}.", {
          instructions: presence(c.instructions_present),
          persona: presence(c.persona_present),
          memoryNotes: c.memory_notes,
          skills: c.skills,
          serviceGrants: c.service_grants,
          accountRead: translateUi(c.account_read ? "yes" : "no"),
        })}
      </p> : <p>{translateUi("Agent configuration has not been checked.")}</p>}
      <p>{translateUi("These facts do not confirm complete business information or a successful customer test. Opening access is your choice. During model setup, customer requests wait briefly or receive a safe setup failure.")}</p>
    </div>
  );
}
