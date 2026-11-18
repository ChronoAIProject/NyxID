import { useEffect, useState, type ReactNode } from "react";
import { initializeDomains } from "../lib/domainBootstrap";
import { DOMAIN_SETS } from "../lib/domains";

/**
 * Blocks the whole app until the `mobile:new-domains` admin flag has selected
 * the production domain set. The splash screen stays up meanwhile (App holds it
 * until its own startup completes). Builds without a second set render at once.
 */
export function DomainGate({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(DOMAIN_SETS.new === null);

  useEffect(() => {
    if (ready) return;
    let active = true;
    void initializeDomains().finally(() => {
      if (active) setReady(true);
    });
    return () => {
      active = false;
    };
  }, [ready]);

  return ready ? children : null;
}
