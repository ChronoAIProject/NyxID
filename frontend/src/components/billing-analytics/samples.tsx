// Loaded only by the browser-test server; normal previews use the live APIs.
import { AnalyticsWorkspace } from "@/pages/admin-usage";
import { useUsageWorkspace } from "@/hooks/use-usage-workspace";
import { Skeleton } from "@/components/ui/skeleton";
import type { AnalyticsLayout } from "@/schemas/usage-analytics";
import { SAMPLE_OPTIONS, sampleAnalytics } from "./sample-data";

export function AnalyticsSamples({ layout }: { layout: AnalyticsLayout }) {
  const userId = `design-review-${layout}`;
  const query = useUsageWorkspace(userId, true);
  if (!query.data) return <Skeleton className="h-96" />;
  return (
    <AnalyticsWorkspace
      key={userId}
      userId={userId}
      initial={query.data}
      initialLayout={layout}
      editable
      sample={sampleAnalytics}
      options={SAMPLE_OPTIONS}
    />
  );
}
