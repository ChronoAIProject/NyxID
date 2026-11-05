import { lazy, Suspense } from "react";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import type { TurnCrmArtifact } from "@/schemas/assistant-nyxagent";

const CrmArtifacts = lazy(() => import("@/components/crm/artifacts"));

export function CrmArtifactBlock({
  artifacts,
}: {
  readonly artifacts?: readonly TurnCrmArtifact[];
}) {
  const enabled = useFeature(FEATURE_FLAG.CMS_POC_V1);
  if (!enabled || !artifacts?.length) return null;
  return (
    <Suspense
      fallback={
        <p role="status" className="my-3 text-12 text-text-secondary">
          Loading artifacts…
        </p>
      }
    >
      <CrmArtifacts artifacts={artifacts} />
    </Suspense>
  );
}
