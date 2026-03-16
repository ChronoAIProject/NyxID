import { registerRootComponent } from "expo";
import { bootstrapNotificationInfrastructure } from "./src/lib/notifications/pushNotifications";
import App from "./src/app/App";

bootstrapNotificationInfrastructure();

registerRootComponent(App);

