interface BillingError {
  readonly title: string;
  readonly message: string;
  readonly openBilling: boolean;
}

export function billingError(code: string | null | undefined): BillingError | undefined {
  switch (code) {
    case "insufficient_credits":
      return {
        title: "Not enough credits",
        message: "There aren't enough credits to run this turn. Add credits in Billing & Usage, then try again.",
        openBilling: true,
      };
    case "wallet_suspended":
      return {
        title: "Billing wallet suspended",
        message: "Your billing wallet is suspended. Contact support or check Billing & Usage.",
        openBilling: true,
      };
    case "plan_entitlement_required":
      return {
        title: "Service not included in your plan",
        message: "Your plan does not include this service. Check your plan in Billing & Usage.",
        openBilling: true,
      };
    case "billing_not_configured":
      return {
        title: "Billing setup incomplete",
        message: "Billing setup is incomplete, so NyxBot could not run this turn. Contact support to complete the billing configuration.",
        openBilling: false,
      };
    case "billing_provider_unavailable":
      return {
        title: "Billing service unavailable",
        message: "NyxBot could not verify billing because the billing service is unavailable. Try again shortly; if this continues, contact support.",
        openBilling: false,
      };
    default:
      return undefined;
  }
}
