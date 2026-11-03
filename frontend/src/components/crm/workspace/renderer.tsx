import { JSONUIProvider, Renderer } from "@json-render/react";
import { crmViewRegistry } from "@/components/crm/workspace/registry";
import { CrmViewErrors } from "@/components/crm/workspace/view-errors";
import { CrmViewQueryProvider } from "@/lib/crm/views/query-runner";
import type {
  CrmValidationError,
  CrmViewEnvelope,
} from "@/lib/crm/views/schema";
import { validateCrmViewEnvelope } from "@/lib/crm/views/validation";

export interface CrmViewRendererProps {
  readonly envelope: CrmViewEnvelope;
  readonly ownerId?: string;
  readonly valid?: boolean;
  readonly errors?: readonly CrmValidationError[];
}

export function CrmViewRenderer({
  envelope,
  ownerId,
  valid = true,
  errors = [],
}: CrmViewRendererProps) {
  if (!valid) {
    return <CrmViewErrors errors={errors} />;
  }

  const parsed = validateCrmViewEnvelope(envelope);
  if (!parsed.success) {
    return <CrmViewErrors errors={parsed.errors} />;
  }

  return (
    <CrmViewQueryProvider ownerId={ownerId} queries={parsed.data.queries}>
      <JSONUIProvider registry={crmViewRegistry}>
        <Renderer spec={parsed.data.spec} registry={crmViewRegistry} />
      </JSONUIProvider>
    </CrmViewQueryProvider>
  );
}
