import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import type { GuestReadiness as Readiness } from "@/schemas/assistant-nyxagent";

/** Settings-only: the model check does not grant visitor access to services. */
export function GuestReadiness({ readiness }: { readiness?: Readiness | null }) {
  useUiTranslation();
  if (!readiness) return null;
  const ready = readiness.status === "ready";
  const settingUp = readiness.status === "setting_up";
  return (
    <div role="status" className="space-y-1 text-13">
      <p className={ready || settingUp ? "text-muted-foreground" : "text-warning"}>
        {ready ? translateUi("Model ready for visitors") : settingUp ? translateUi("Setting up for visitors…") : translateUi("Visitor setup needs attention")}
      </p>
      {!ready && readiness.fix ? <p className="text-muted-foreground">{readiness.fix}</p> : null}
      {ready ? <p className="text-text-tertiary">{translateUi("Service permissions still apply. Visitor usage is billed to you.")}</p> : null}
    </div>
  );
}
