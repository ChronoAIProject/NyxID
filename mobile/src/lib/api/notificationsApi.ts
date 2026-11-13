import { requestJson } from "./http";
import { createNotificationsApi } from "./notificationsTransport";

export const notificationsApi = createNotificationsApi(requestJson);
