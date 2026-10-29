import { requestJson } from "./http";
import { createAutomationsApi } from "./automationsTransport";

export const automationsApi = createAutomationsApi(requestJson);
