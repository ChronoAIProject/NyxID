// Changes on account admission/logout, not ordinary access-token refresh.
let generation = 0;
export function getAuthEpoch(): number {
  return generation;
}
export function advanceAuthEpoch(): void {
  generation += 1;
}
