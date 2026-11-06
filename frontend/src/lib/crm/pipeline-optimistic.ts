import type { CrmRecordRow } from "@/schemas/crm";

export interface CrmOptimisticPipelineMove {
  readonly record: CrmRecordRow;
  readonly from: string;
  readonly to: string;
}

export function applyOptimisticPipelineRows(
  rows: readonly CrmRecordRow[],
  stage: string,
  moves: Readonly<Record<string, CrmOptimisticPipelineMove>>,
  includeMove: (move: CrmOptimisticPipelineMove) => boolean = () => true,
): CrmRecordRow[] {
  const retained = rows.filter((row) => {
    const move = moves[row.id];
    return !move || (move.to === stage && includeMove(move));
  });
  const ids = new Set(retained.map((row) => row.id));
  for (const move of Object.values(moves)) {
    if (move.to === stage && includeMove(move) && !ids.has(move.record.id)) {
      retained.push(move.record);
      ids.add(move.record.id);
    }
  }
  return retained;
}
