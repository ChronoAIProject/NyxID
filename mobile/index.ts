import { registerRootComponent } from "expo";
import { bootstrapNotificationInfrastructure } from "./src/lib/notifications/pushNotifications";
import AppRoot from "./src/app/AppRoot";

bootstrapNotificationInfrastructure();

registerRootComponent(AppRoot);
