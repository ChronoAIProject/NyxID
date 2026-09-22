import type { ChannelPlatformDescriptor } from "@/types/channels";

export function parseChannelBotSetupSearch(search: Record<string, unknown>): {
  label?: string;
  target_org_id?: string;
  request_id?: string;
} {
  return {
    label:
      typeof search.label === "string" ? search.label.slice(0, 128) : undefined,
    target_org_id:
      typeof search.target_org_id === "string"
        ? search.target_org_id
        : undefined,
    request_id:
      typeof search.request_id === "string" &&
      /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
        search.request_id,
      )
        ? search.request_id
        : undefined,
  };
}

export function parseChannelBotSetupPageSearch(
  search: Record<string, unknown>,
) {
  const fields = Object.fromEntries(
    Object.entries(search)
      .filter(
        ([name, value]) =>
          /^[a-z][a-z0-9_]{0,63}$/.test(name) &&
          (typeof value === "string" ||
            typeof value === "number" ||
            typeof value === "boolean"),
      )
      .map(([name, value]) => [name, String(value)]),
  );
  return { ...fields, ...parseChannelBotSetupSearch(fields) };
}

/** Preserve long numeric IDs and accept strings quoted by the router's serializer. */
export function channelBotSetupUrlValues(
  search: string,
): Record<string, string> {
  return Object.fromEntries(
    [...new URLSearchParams(search)].map(([name, value]) => {
      if (value.startsWith('"')) {
        try {
          const decoded: unknown = JSON.parse(value);
          if (typeof decoded === "string") return [name, decoded];
        } catch {
          /* Plain query values need not be JSON. */
        }
      }
      return [name, value];
    }),
  );
}

export function channelBotSetupPrefill(
  search: string,
  descriptor: ChannelPlatformDescriptor,
) {
  const values = channelBotSetupUrlValues(search);
  return Object.fromEntries(
    descriptor.registration.fields.flatMap(({ name }) =>
      Object.hasOwn(values, name) ? [[name, values[name]!]] : [],
    ),
  );
}

// Form inputs are text, including numeric IDs and credentials that resemble JSON.
// Quote them before the router's default JSON parser can change their values.
export const channelBotSetupRewrite = {
  input: ({ url }: { url: URL }) => {
    if (/^\/channel-bots\/connect\/[^/]+\/?$/.test(url.pathname)) {
      for (const [name, value] of Object.entries(
        channelBotSetupUrlValues(url.search),
      )) {
        url.searchParams.set(name, JSON.stringify(value));
      }
    }
    return url;
  },
};
