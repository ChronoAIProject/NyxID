import { Link } from "@tanstack/react-router";
import type { CrmObject } from "@/schemas/crm";
import { CrmViewsPanel } from "./workspace/crm-views-panel";

export function CrmArtifactsPanel({
  ownerId,
  objects,
}: {
  readonly ownerId?: string;
  readonly objects: {
    data?: readonly CrmObject[];
    isPending: boolean;
    isError: boolean;
  };
}) {
  const datasets = objects.data?.filter((object) => !object.seeded) ?? [];
  return (
    <div className="space-y-6">
      <section aria-label="Custom datasets" className="space-y-3">
        <h2 className="type-body-strong">Custom datasets</h2>
        {objects.isPending ? (
          <p role="status">Loading datasets…</p>
        ) : objects.isError ? (
          <p role="alert">Could not load datasets.</p>
        ) : datasets.length ? (
          <ul className="space-y-2">
            {datasets.map((object) => (
              <li key={object.id}>
                <Link
                  to="/assistant/crm"
                  search={{
                    owner: ownerId,
                    object: object.key,
                    record: undefined,
                    view: undefined,
                    surface: undefined,
                  }}
                  className="text-13 text-primary underline underline-offset-4"
                >
                  {object.plural_label}
                </Link>
              </li>
            ))}
          </ul>
        ) : (
          <p className="text-12 text-text-secondary">No custom datasets yet.</p>
        )}
      </section>
      <section aria-label="Saved views" className="space-y-3">
        <h2 className="type-body-strong">Saved views</h2>
        <CrmViewsPanel ownerId={ownerId} />
      </section>
    </div>
  );
}
