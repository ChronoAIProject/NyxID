import { OAuthConsentPage } from "./oauth-consent";
import {
  previewAgentSearch,
  previewSearch,
  previewServices,
} from "./oauth-consent-preview-data";

/** Dev-only consent preview. `?app=claude` shows an agent (MCP) sign-in. */
export function OAuthConsentPreviewPage() {
  const agent =
    new URLSearchParams(window.location.search).get("app") === "claude";
  return (
    <OAuthConsentPage
      preview={{
        search: agent ? previewAgentSearch : previewSearch,
        services: previewServices,
        email: "alex@example.com",
      }}
    />
  );
}
