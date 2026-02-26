import { Capacitor } from "@capacitor/core";

export const isNative = Capacitor.isNativePlatform();
export const isWeb = !isNative;
export const platform = Capacitor.getPlatform() as "web" | "ios" | "android";
