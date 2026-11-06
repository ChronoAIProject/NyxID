import { CrmOwnerContext } from "@/lib/crm/owner-context";
import { JSONUIProvider, Renderer } from "@json-render/react";
import { crmViewRegistry } from "@/components/crm/workspace/registry";
import { CrmViewErrors } from "@/components/crm/workspace/view-errors";
import { CrmViewQueryProvider } from "@/lib/crm/views/query-runner";
import type { CrmValidationError } from "@/lib/crm/views/schema";
import { validateCrmViewEnvelope } from "@/lib/crm/views/validation";

export interface CrmViewRendererProps {
  readonly envelope: unknown;
  readonly ownerId?: string;
  readonly valid?: boolean;
  readonly errors?: readonly CrmValidationError[];
  readonly recordId?: string;
  readonly onRecordChange?: (id: string | undefined) => void;
}

export function CrmViewRenderer({
  envelope,
  ownerId,
  valid = true,
  errors = [],
  recordId: selectedRecordId,
  onRecordChange,
}: CrmViewRendererProps) {
  const [previewRecordId, setPreviewRecordId] = useState<string>();
  const recordId = onRecordChange ? selectedRecordId : previewRecordId;
  const setRecordId = onRecordChange ?? setPreviewRecordId;
  if (!valid) {
    return <CrmViewErrors errors={errors} />;
  }

  const parsed = validateCrmViewEnvelope(envelope);
  if (!parsed.success) {
    return <CrmViewErrors errors={parsed.errors} />;
  }

  return (
    <CrmOwnerContext value={ownerId}>
      <CrmViewQueryProvider ownerId={ownerId} queries={parsed.data.queries}>
        <CrmRecordNavigation value={setRecordId}>
          <JSONUIProvider registry={crmViewRegistry}>
            <Renderer spec={parsed.data.spec} registry={crmViewRegistry} />
          </JSONUIProvider>
          {recordId ? (
            <CrmRecordSheet
              key={`${ownerId}:${recordId}`}
              recordId={recordId}
              ownerId={ownerId}
              open={Boolean(recordId)}
              onOpenChange={(open) => {
                if (!open) setRecordId(undefined);
              }}
              onSelectRecord={setRecordId}
            />
          ) : null}
        </CrmRecordNavigation>
      </CrmViewQueryProvider>
    </CrmOwnerContext>
  );
}
import { useState } from "react";
import { CrmRecordSheet } from "@/components/crm/record-sheet";
import { CrmRecordNavigation } from "@/lib/crm/record-navigation";
