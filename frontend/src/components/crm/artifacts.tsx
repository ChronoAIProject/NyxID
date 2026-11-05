import type { ReactNode } from "react";
import { ApiError } from "@/lib/api-client";
import { useCrmAggregate, useCrmObject, useCrmRecord } from "@/hooks/use-crm";
import { useCrmView } from "@/hooks/use-crm-views";
import { CrmViewRenderer } from "./workspace/renderer";
import type { TurnCrmArtifact } from "@/schemas/assistant-nyxagent";
import { renderCrmCell } from "@/lib/crm/field-kinds";
import { CrmOwnerContext } from "@/lib/crm/owner-context";

function Frame({
  title,
  href,
  children,
}: {
  readonly title: string;
  readonly href: string;
  readonly children: ReactNode;
}) {
  return (
    <section
      className="my-3 overflow-hidden rounded-lg border border-border bg-card"
      aria-label="CRM artifact"
    >
      <div className="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
        <h3 className="truncate text-13 font-semibold">{title}</h3>
        <a
          href={href}
          className="shrink-0 text-12 text-primary underline underline-offset-4"
        >
          Open
        </a>
      </div>
      <div className="max-h-96 overflow-auto p-4">{children}</div>
    </section>
  );
}
function State({
  error,
  loading,
}: {
  readonly error?: unknown;
  readonly loading?: boolean;
}) {
  if (loading)
    return (
      <p role="status" className="my-3 text-12 text-text-secondary">
        Loading artifact…
      </p>
    );
  const unavailable =
    error instanceof ApiError && [401, 403, 404, 410].includes(error.status);
  return (
    <p role="alert" className="my-3 text-12 text-text-secondary">
      {unavailable
        ? "This artifact is no longer available"
        : "Could not load this artifact. It will retry when the connection is restored."}
    </p>
  );
}
function href(artifact: TurnCrmArtifact) {
  const query = new URLSearchParams({ owner: artifact.owner_id });
  if (artifact.kind === "view") query.set("view", artifact.id);
  else {
    query.set("object", artifact.object_key ?? "");
    if (artifact.kind === "record") query.set("record", artifact.id);
  }
  return `/assistant/crm?${query}`;
}
function ViewArtifact({ artifact }: { readonly artifact: TurnCrmArtifact }) {
  const view = useCrmView(artifact.id, artifact.owner_id);
  if (view.isError) return <State error={view.error} />;
  if (!view.data) return <State loading />;
  return (
    <Frame title={view.data.title} href={href(artifact)}>
      <CrmViewRenderer
        envelope={view.data.envelope}
        ownerId={artifact.owner_id}
        valid={view.data.valid}
        errors={view.data.errors}
      />
    </Frame>
  );
}
function RecordArtifact({ artifact }: { readonly artifact: TurnCrmArtifact }) {
  const record = useCrmRecord(artifact.id, artifact.owner_id);
  const object = useCrmObject(record.data?.object_key, artifact.owner_id);
  if (record.isError || object.isError)
    return <State error={record.error ?? object.error} />;
  if (!record.data || !object.data) return <State loading />;
  const fields = object.data.fields
    .filter((f) => !f.archived && f.type !== "record")
    .slice(0, 5);
  const titleField = object.data.fields.find(
    (f) => f.type === "text" && f.required,
  );
  const title = titleField
    ? String(record.data.values[titleField.key] ?? object.data.label)
    : object.data.label;
  return (
    <Frame
      title={title}
      href={href({ ...artifact, object_key: record.data.object_key })}
    >
      <p className="mb-2 text-11 text-text-tertiary">
        {object.data.label}
        {record.data.archived ? " · Archived" : ""}
      </p>
      <CrmOwnerContext.Provider value={artifact.owner_id}>
        <dl className="space-y-2 text-12">
          {fields.map((f) => (
            <div key={f.key} className="grid grid-cols-2 gap-3">
              <dt className="text-text-secondary">{f.label}</dt>
              <dd className="line-clamp-3 break-words">
                {renderCrmCell(f, record.data!.values[f.key])}
              </dd>
            </div>
          ))}
        </dl>
      </CrmOwnerContext.Provider>
      {fields.length === 0 ? (
        <p className="text-12 text-text-secondary">No key fields to display.</p>
      ) : null}
    </Frame>
  );
}
function ObjectArtifact({ artifact }: { readonly artifact: TurnCrmArtifact }) {
  const object = useCrmObject(
    artifact.object_key ?? undefined,
    artifact.owner_id,
  );
  const count = useCrmAggregate(
    object.data
      ? {
          object: object.data.key,
          filters: [],
          measure: { op: "count" },
          order: "value_desc",
        }
      : undefined,
    artifact.owner_id,
  );
  if (object.isError || count.isError)
    return <State error={object.error ?? count.error} />;
  if (!object.data || !count.data) return <State loading />;
  const total = count.data.points.reduce(
    (sum, point) => sum + (point.value ?? 0),
    0,
  );
  return (
    <Frame title={object.data.plural_label} href={href(artifact)}>
      <p className="text-12 text-text-secondary">
        {total === 0 ? "No records yet." : `${total.toLocaleString()} records`}
      </p>
    </Frame>
  );
}
export default function CrmArtifacts({
  artifacts,
}: {
  readonly artifacts: readonly TurnCrmArtifact[];
}) {
  return (
    <div>
      {artifacts
        .slice(0, 8)
        .map((artifact) =>
          artifact.kind === "view" ? (
            <ViewArtifact key={`view:${artifact.id}`} artifact={artifact} />
          ) : artifact.kind === "record" ? (
            <RecordArtifact key={`record:${artifact.id}`} artifact={artifact} />
          ) : (
            <ObjectArtifact key={`object:${artifact.id}`} artifact={artifact} />
          ),
        )}
    </div>
  );
}
