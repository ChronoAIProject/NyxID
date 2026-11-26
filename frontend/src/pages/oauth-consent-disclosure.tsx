import { useId } from "react";
import { Badge } from "@/components/ui/badge";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import type {
  ConsentDisclosure,
  ServiceConsentDisclosure,
} from "@/schemas/oauth-consent";

export interface ConsentSummaryService {
  readonly id: string;
  readonly name: string;
  readonly secondary?: string;
  readonly orgName: string | null;
}

interface ServiceAccess {
  readonly allow_all_services: boolean;
  readonly allowed_service_ids: readonly string[];
}

export type ConsentProvenance =
  | "required"
  | "preselected"
  | "preselected_locked"
  | "selected"
  | "already"
  | "retained";

export function ConsentSelectionLabel({
  provenance,
}: {
  readonly provenance: ConsentProvenance;
}) {
  useUiTranslation();
  switch (provenance) {
    case "required":
      return translateUi("Required by app");
    case "preselected":
      return translateUi("Preselected by app");
    case "preselected_locked":
      return translateUi("Preselected by app (locked)");
    case "selected":
      return translateUi("Selected by you");
    case "already":
      return translateUi("Already authorized");
    case "retained":
      return translateUi("Stays authorized");
  }
}

function credentialKind(kind?: ServiceConsentDisclosure["credential_kind"]) {
  switch (kind) {
    case "oauth":
      return translateUi("OAuth credential");
    case "api_key":
      return translateUi("API key");
    case "platform":
      return translateUi("Platform credential");
    case "node_managed":
      return translateUi("Node-managed credential");
    case "other":
      return translateUi("Other credential");
    default:
      return null;
  }
}

export function ServiceConsentDetails({
  disclosure,
  orgName,
  id,
}: {
  readonly disclosure?: ServiceConsentDisclosure;
  readonly orgName: string | null;
  readonly id?: string;
}) {
  useUiTranslation();
  const source = disclosure?.credential_source;
  const org = source?.type === "org" ? source.org_name : orgName;
  const kind = credentialKind(disclosure?.credential_kind);
  const operations = disclosure?.known_operations;
  return (
    <div
      id={id}
      className="mt-1 space-y-1 break-words text-13 text-muted-foreground"
    >
      <p>
        {org === null
          ? translateUi("Personal")
          : translateUi("Organization · {{value1}}", { value1: org })}
        {kind && <> · {kind}</>}
      </p>
      {disclosure?.credential_kind === "oauth" &&
      disclosure.upstream_scopes?.length ? (
        <div>
          <p className="font-medium">{translateUi("Upstream permissions")}</p>
          <ul className="list-disc space-y-1 ps-5">
            {disclosure.upstream_scopes.map((scope) => (
              <li key={scope.scope}>
                {scope.known ? (
                  scope.label
                ) : (
                  <>
                    <code className="break-all">{scope.scope}</code>{" "}
                    <span>({translateUi("Unrecognized permission")})</span>
                  </>
                )}
                {scope.sensitive && (
                  <Badge variant="secondary" className="ms-1 align-middle">
                    {translateUi("Sensitive")}
                  </Badge>
                )}
                {scope.known && <p>{scope.description}</p>}
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <p>
          {disclosure?.credential_kind === "api_key"
            ? translateUi(
                "The upstream provider controls what this API key can do. Its permissions are unavailable here.",
              )
            : translateUi(
                "Upstream permission details are unavailable. Full service access uses the connection’s permissions.",
              )}
        </p>
      )}
      {operations && (
        <p>
          {translateUi(
            "Known published operations include {{read}} read, {{write}} write, {{remove}} delete, and {{unclassified}} unclassified.",
            {
              read: operations.read,
              write: operations.write,
              remove: operations.delete,
              unclassified: operations.unclassified,
            },
          )}{" "}
          {translateUi("These counts do not limit access.")}
        </p>
      )}
      {disclosure?.capabilities.model_calls === true && (
        <p>
          {translateUi(
            "Catalog metadata lists model calls. The app may send data to the model provider.",
          )}
        </p>
      )}
      {disclosure?.capabilities.binary_upload === true && (
        <p>{translateUi("Catalog metadata lists binary upload support.")}</p>
      )}
    </div>
  );
}

export type ConsentInventoryState = "loading" | "failed" | "ready";

function unavailableServiceLabel(state: ConsentInventoryState) {
  if (state === "loading") return translateUi("Loading services...");
  if (state === "failed")
    return translateUi("Service details could not be loaded.");
  return translateUi("Service details unavailable");
}

export function ConsentUnavailableService({
  state,
}: {
  readonly state: ConsentInventoryState;
}) {
  useUiTranslation();
  return unavailableServiceLabel(state);
}

export function ConsentServiceSummary({
  id,
  access,
  services,
  disclosures,
  inventoryState,
  provenance,
  retained,
  retainedAll = false,
}: {
  readonly id: string;
  readonly access: ServiceAccess;
  readonly services: ReadonlyMap<string, ConsentSummaryService>;
  readonly disclosures: ReadonlyMap<string, ServiceConsentDisclosure>;
  readonly inventoryState: ConsentInventoryState;
  readonly provenance: (id: string) => readonly ConsentProvenance[];
  readonly retained?: readonly string[];
  readonly retainedAll?: boolean;
}) {
  useUiTranslation();
  const includedIds = access.allow_all_services
    ? [...services.keys()]
    : access.allowed_service_ids;
  const unavailable = unavailableServiceLabel(inventoryState);
  const retainedNames = retained?.map(
    (serviceId) => services.get(serviceId)?.name ?? unavailable,
  );
  const describesRetained = retainedAll || Boolean(retainedNames?.length);
  return (
    <section
      id={id}
      aria-labelledby={`${id}-heading`}
      className="space-y-2 text-14 text-foreground"
    >
      <h2 id={`${id}-heading`} className="type-card-title">
        {translateUi("Selected service access")}
      </h2>
      <p id={`${id}-description`} aria-live="polite" aria-atomic="true">
        {access.allow_all_services
          ? translateUi("All current and future services are included.")
          : access.allowed_service_ids.length === 0
            ? translateUi("No services are included in this request.")
            : access.allowed_service_ids.length === 1
              ? translateUi("Full access to one selected service.")
              : translateUi("Full access to {{count}} selected services.", {
                  count: access.allowed_service_ids.length,
                })}
        {describesRetained && (
          <> {translateUi("Earlier authorization stays in effect.")}</>
        )}
      </p>
      {(access.allow_all_services || access.allowed_service_ids.length > 0) && (
        <p>
          {translateUi(
            "Full service access can include reading, creating, updating or deleting data. Approval prompts depend on your configured policies.",
          )}
        </p>
      )}
      {access.allow_all_services && inventoryState !== "ready" && (
        <p>{unavailable}</p>
      )}
      {includedIds.length > 0 && (
        <ul className="space-y-2">
          {includedIds.map((serviceId) => {
            const service = services.get(serviceId);
            const labels = provenance(serviceId);
            return (
              <li
                key={serviceId}
                className="break-words rounded-md bg-overlay px-3 py-2"
              >
                <p className="font-medium">{service?.name ?? unavailable}</p>
                {service?.secondary && (
                  <p className="text-13 text-muted-foreground">
                    {service.secondary}
                  </p>
                )}
                {labels.length > 0 && (
                  <div className="flex flex-wrap gap-1">
                    {labels.map((label) => (
                      <Badge key={label} variant="secondary">
                        <ConsentSelectionLabel provenance={label} />
                      </Badge>
                    ))}
                  </div>
                )}
                {service && (
                  <ServiceConsentDetails
                    disclosure={disclosures.get(serviceId)}
                    orgName={service.orgName}
                  />
                )}
              </li>
            );
          })}
        </ul>
      )}
      {retainedAll ? (
        <p>
          {translateUi(
            "Previously authorized access to all services stays authorized.",
          )}
        </p>
      ) : (
        Boolean(retainedNames?.length) && (
          <p>
            {translateUi(
              "These services stay authorized from an earlier approval. {{services}}",
              {
                services: retainedNames?.join(", "),
              },
            )}
          </p>
        )
      )}
    </section>
  );
}

function tokenDuration(seconds: number, language: string | undefined) {
  const units = [
    ["day", 86400],
    ["hour", 3600],
    ["minute", 60],
    ["second", 1],
  ] as const;
  const [unit, unitSeconds] =
    units.find(([, size]) => seconds % size === 0) ?? units[3];
  return new Intl.NumberFormat(language, {
    style: "unit",
    unit,
    unitDisplay: "long",
  }).format(seconds / unitSeconds);
}

export function ConsentDuration({
  disclosure,
  offline,
  broker,
  className,
}: {
  readonly disclosure?: ConsentDisclosure;
  readonly offline: boolean;
  readonly broker: boolean;
  readonly className?: string;
}) {
  const { i18n } = useUiTranslation();
  const language = i18n.resolvedLanguage;
  const id = useId();
  return (
    <section aria-labelledby={id} className={className}>
      <h2 id={id} className="type-card-title text-foreground">
        {translateUi("Duration and revocation")}
      </h2>
      <div className="mt-2 space-y-2 text-14 text-muted-foreground">
        {disclosure && (
          <p>
            {translateUi(
              "The configured lifetime for standard access tokens is {{duration}}.",
              {
                duration: tokenDuration(
                  disclosure.access_token_ttl_secs,
                  language,
                ),
              },
            )}
          </p>
        )}
        {offline && (
          <p>
            <strong>{translateUi("Long-lived access")}</strong> (
            <code className="break-all">offline_access</code>).{" "}
            <span>
              {translateUi(
                "Lets the app refresh tokens without asking you to log in every time.",
              )}
            </span>
          </p>
        )}
        {broker && (
          <p>
            <strong>{translateUi("Durable broker access")}</strong> (
            <code className="break-all">urn:nyxid:scope:broker_binding</code>).{" "}
            <span>
              {translateUi(
                "Lets the app hold a durable NyxID credential that can act as you.",
              )}
            </span>
          </p>
        )}
        {disclosure?.broker_access_token_ttl_secs && (
          <p>
            {translateUi(
              "The configured lifetime for broker access tokens is {{duration}}. This applies to apps using the broker token flow.",
              {
                duration: tokenDuration(
                  disclosure.broker_access_token_ttl_secs,
                  language,
                ),
              },
            )}
          </p>
        )}
        {disclosure && (
          <p>
            {translateUi(
              "When issued, each refresh token lasts {{duration}}. The app may keep access by refreshing before expiry.",
              {
                duration: tokenDuration(
                  disclosure.refresh_token_ttl_secs,
                  language,
                ),
              },
            )}
          </p>
        )}
        <p>
          {translateUi(
            "You can revoke app access from Authorized Applications. The app will need your consent again to regain access.",
          )}
        </p>
      </div>
    </section>
  );
}
