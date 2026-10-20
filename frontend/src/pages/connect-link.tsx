import { useParams } from "@tanstack/react-router";
import {
  ConnectLinkContent,
  ConnectLinkReturnPage,
  ConnectLinkDetailRow,
  RequestDetails,
  TerminalPanel,
} from "@/components/connect-link/connect-link-content";

/** Standalone hosted connection route. The embedded chat flow uses the same content component. */
export function ConnectLinkPage() {
  const { token } = useParams({ strict: false }) as { token: string };
  return <ConnectLinkContent token={token} />;
}

export {
  ConnectLinkDetailRow,
  ConnectLinkReturnPage,
  RequestDetails,
  TerminalPanel,
};
