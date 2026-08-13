const AGENT_POC_PLAN_BLOCK_SUFFIX = "-agent-poc-plan";

export function isAgentPocPlanBlockId(blockId: string): boolean {
  return blockId.endsWith(AGENT_POC_PLAN_BLOCK_SUFFIX);
}
