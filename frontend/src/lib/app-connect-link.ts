import type { AppConnectLink } from "@/schemas/app-connect-links";

export function requirementsReady(link: AppConnectLink, now: number): boolean {
  return link.items.every(
    (item) =>
      item.optional ||
      (item.state === "met" &&
        (!item.valid_until || new Date(item.valid_until).getTime() > now) &&
        (link.origin !== "authorize" ||
          !item.validated_at ||
          now - Date.parse(item.validated_at) <= 60_000)),
  );
}

export function appConnectCapabilityStorageKey(linkId: string): string {
  return `nyxid:app-connect-capability:${linkId}`;
}
