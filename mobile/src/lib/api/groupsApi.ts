import { requestJson } from "./http";
import { createGroupsApi } from "./groupsTransport";

export const groupsApi = createGroupsApi(requestJson);
