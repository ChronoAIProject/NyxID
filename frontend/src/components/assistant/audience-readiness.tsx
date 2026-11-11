import { useUiTranslation } from "@/i18n/use-ui-translation";
import type { AgentConfiguration } from "@/schemas/assistant-nyxagent";

/** Information only: owners can open access even while configuration is incomplete. */
export function AudienceReadiness({ configuration: c }: { configuration?: AgentConfiguration | null }) {
  const { t } = useUiTranslation();
  return (
    <div className="space-y-1" role="status">
      {c ? <p>
        {t("Instructions: {{instructions}}; persona: {{persona}}; memory notes: {{memoryNotes}}; skills: {{skills}}; service grants: {{serviceGrants}}; account read: {{accountRead}}.", {
          instructions: t(c.instructions_present ? "present" : "missing"),
          persona: t(c.persona_present ? "present" : "missing"),
          memoryNotes: c.memory_notes,
          skills: c.skills,
          serviceGrants: c.service_grants,
          accountRead: t(c.account_read ? "yes" : "no"),
        })}
      </p> : <p>{t("Agent configuration has not been checked.")}</p>}
      <p>{t("These facts do not confirm complete business information or a successful customer test. Opening access is your choice. During model setup, customer requests wait briefly or receive a safe setup failure.")}</p>
    </div>
  );
}
