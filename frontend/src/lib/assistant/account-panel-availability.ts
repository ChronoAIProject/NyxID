import { isBillingAvailable, type User } from "@/types/api";
import type { AccountPanel } from "./account-panel-search";

export function accountPanelAllowed(panel: AccountPanel, user: User) {
  return panel === "billing" ? isBillingAvailable(user) : true;
}
