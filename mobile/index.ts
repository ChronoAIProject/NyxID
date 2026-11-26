import { registerRootComponent } from "expo";
import { bootstrapNotificationInfrastructure } from "./src/lib/notifications/pushNotifications";
import { installMobileCrashReporting } from "./src/lib/crashReporting/runtime";
import AppRoot from "./src/app/AppRoot";

bootstrapNotificationInfrastructure();
installMobileCrashReporting();

registerRootComponent(AppRoot);
