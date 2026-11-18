import { requestJson } from "./http";
import { createReplyReportApi } from "./replyReportTransport";

export const replyReportApi = createReplyReportApi(requestJson);
