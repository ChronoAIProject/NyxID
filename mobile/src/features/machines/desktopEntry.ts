import { z } from "zod";
import type { MachineDesktopParams } from "./desktop/desktopRoutes";

export type MachineDesktopRouteParams = MachineDesktopParams & {
  conversation_id?: string;
  context_id?: string;
};
const selector = z
  .string()
  .min(1)
  .max(256)
  .regex(/^[a-zA-Z0-9_-]+$/);
const routeSchema = z.object({
  nodeId: selector,
  conversationId: selector.optional(),
  contextId: selector.optional(),
  display: z.enum(["secure", "dev"]).optional(),
});
export function desktopRouteParams(
  params: MachineDesktopRouteParams,
): MachineDesktopParams {
  if (
    params.conversationId &&
    params.conversation_id &&
    params.conversationId !== params.conversation_id
  )
    throw new Error("Conflicting desktop conversation selectors.");
  if (
    params.contextId &&
    params.context_id &&
    params.contextId !== params.context_id
  )
    throw new Error("Conflicting desktop context selectors.");
  return routeSchema.parse({
    nodeId: params.nodeId,
    conversationId: params.conversationId ?? params.conversation_id,
    contextId: params.contextId ?? params.context_id,
    display: params.display,
  });
}
export const desktopMetadataSchema = z.object({
  node_id: z.string(),
  session_id: z.string(),
  conversation_id: z.string().nullable(),
  context_id: z.string().nullish(),
  display: z.enum(["secure", "dev"]).default("secure"),
  status: z.string(),
  reason: z.string().nullish(),
});
export function desktopEntryParams(
  row: z.infer<typeof desktopMetadataSchema>,
  conversationId?: string,
): MachineDesktopParams {
  return desktopRouteParams({
    nodeId: row.node_id,
    conversationId: row.conversation_id ?? conversationId,
    contextId: row.context_id ?? undefined,
    display: row.display,
  });
}
