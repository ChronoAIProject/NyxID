import { requestJson } from "./http";
import { createCollaborationApi } from "./collaborationTransport";

export const collaborationApi = createCollaborationApi(requestJson);
