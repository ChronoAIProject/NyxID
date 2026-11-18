import { requestJson } from "./http";
import { createBillingApi } from "../../features/billing/billingTransport";

export const billingApi = createBillingApi(requestJson);
