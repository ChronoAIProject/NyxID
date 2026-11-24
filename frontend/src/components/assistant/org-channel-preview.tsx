import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import type { OrgChannelPreview, OrgChannelSettings } from "@/schemas/org-specialist-channel";
import { GuestReadiness } from "./guest-readiness";
import { AudienceReadiness } from "./audience-readiness";

export function GuestServices({ services }: { readonly services: OrgChannelSettings["effective_guest_services"] }) {
  useUiTranslation();
  const labels: Record<string, string> = {
    none: "No guest access", read: "Look things up only", use: "Use, but not change or delete", all: "Everything this agent can do",
  };
  return services.length ? <ul>{services.map((service) => <li key={service.id}>
    {service.label}: {translateUi(labels[service.access] ?? service.access)}
    {service.limited_operations ? ` · ${translateUi("Selected operations only")}` : ""}
  </li>)}</ul> : <p>{translateUi("Model-only chat")}</p>;
}

export function OrgChannelDisclosure({ preview }: { readonly preview: OrgChannelPreview }) {
  useUiTranslation();
  return <div className="space-y-3 type-secondary">
    <p>{translateUi(preview.description)}</p>
    <p>{translateUi("Guests cannot use account tools, approvals, nodes, machines, saved logins or private memory.")}</p>
    {preview.guest_enabled && <>
      <p>{translateUi("Guests can use")}</p>
      <GuestServices services={preview.effective_guest_services} />
    </>}
    <GuestReadiness readiness={preview.audience_readiness.guest_readiness} />
    <AudienceReadiness configuration={preview.audience_readiness.configuration} />
    {preview.health ? <p role="status">{translateUi(preview.health.text)}</p> : null}
  </div>;
}
