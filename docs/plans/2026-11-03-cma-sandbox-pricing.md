# CMA Sandbox pricing and NyxID integration proposal

Status: **Draft for review. No prices are approved or activated.**

CMA now records allocated CPU, RAM, and retained disk over Sandbox lifecycle
intervals. This proposal puts the commercial decision alongside NyxID's billing
documentation. It does not change NyxID runtime behavior or Lago configuration.
CMA's production billing exporter remains disabled.

## Proposed USD prices

These are starting prices for discussion, not measured infrastructure costs or a
margin forecast. Validate them against regional compute/storage costs, idle pool
capacity, backups, traffic and support before approval. Rates are chosen so their
integer-unit prices fit NyxID's twelve-decimal credit precision exactly.

| Resource | Proposed rate, before tax | Allocation with runtime attached | Paused after runtime release |
| --- | ---: | --- | --- |
| CPU | $0.036 per vCPU-hour | Allocated vCPU × duration | $0 |
| RAM | $0.0036864 per GiB-hour | Allocated GiB × duration | $0 |
| Retained disk | $0.000144 per GiB-hour | Allocated GiB × duration | Continues until storage is released |

RAM is approximately $0.0037/GiB-hour for display; **$0.0036864 is the exact
proposed rate**. Disk is $0.10368/GiB for 720 hours. A month is not a fixed billing
unit: actual elapsed seconds determine usage. CPU and RAM accrue while a runtime
is bound, including installation and pause cleanup. Clicking Pause does not stop
accrual before runtime release is recorded. Disk stops on the durable home-release
receipt. Warm capacity is not attributed to a customer before Sandbox admission.

These quantities measure allocated capacity, not sampled CPU utilization,
resident memory, or filesystem bytes. The proposed tariff has no additional
uptime charge, minimum-hour charge, or per-request charge. AI model usage is a
separate product and is not included. Network, backup and snapshot charges are
not specified by this proposal.

### Worked example: 2 vCPU, 8 GiB RAM, 64 GiB disk

| Scenario | Exact pre-tax amount | Approximate displayed amount |
| --- | ---: | ---: |
| Active for one hour | $0.1107072 | $0.1107 |
| Paused with disk retained for one hour | $0.009216 | $0.0092 |
| Active continuously for 30 days / 720 hours | $79.709184 | $79.71 |
| Active 8 hours/day and paused 16 hours/day for 30 days | $30.993408 | $30.99 |
| Paused with disk retained for 30 days / 720 hours | $6.63552 | $6.64 |

The active hourly amount is `2 × 0.036 + 8 × 0.0036864 + 64 × 0.000144`.
The mixed-month example uses 240 compute hours and 720 disk hours. These are
illustrations, not invoice promises; Lago owns applicable taxes, invoice rounding
and settlement. Preserve exact decimals until the provider's documented rounding
boundary.

## Lago mapping

Use dedicated SUM metrics (`aggregation_type: sum_agg`, `field_name: quantity`)
with standard charges on the owner's existing subscription. Never reuse NyxID's
machine runtime SKU: it uses running seconds with paused storage free, which is
a different contract from CMA's retained-disk accounting.

| Metric code | Integer quantity | Exact proposed USD per integer unit |
| --- | --- | ---: |
| `cma_sandbox_cpu_millis_seconds` | millicore-seconds | `0.000000010000` |
| `cma_sandbox_memory_mib_seconds` | MiB-seconds | `0.000000001000` |
| `cma_sandbox_disk_gib_seconds` | GiB-seconds | `0.000000040000` |

Conversions: divide the CPU vCPU-hour price by 3,600,000, the RAM GiB-hour price
by 3,686,400, and the disk GiB-hour price by 3,600. The table yields respectively
10,000, 1,000 and 40,000 picocredits per integer unit when one credit is one USD.
Use decimal strings, never binary floating point. Other currencies and wallet
exchange rates require a separate approved conversion.

Lago updates must preserve unrelated charge IDs and the full existing charge
array. Verify effective subscription charges as well as plan defaults; existing
subscriptions may override the plan. Keep existing taxes and discounts intact.
Do not create duplicate subscriptions or change customer ownership to make a
configuration pass. The current CMA exporter requires an active calendar-billed
subscription and effective UTC customer timezone.

## What belongs in CMA, Lago, and NyxID

| Responsibility | Current owner and behavior |
| --- | --- |
| Allocation history and exact quantities | CMA's retained usage ledger and owner/admin usage APIs |
| Meter delivery and retry identity | CMA's immutable outbox; stable metric-specific Lago transaction IDs |
| Authoritative invoice prices | Lago charges; this document proposes their values |
| Current bill and finalized invoices | NyxID reads the existing customer's Lago subscription |
| Reconciled wallet availability | NyxID subtracts provider current usage from the provider's settled wallet balance |
| NyxID internal usage rows, grants and allowances | Requires an explicit CMA-to-NyxID settlement integration |
| Prepaid admission and ongoing spending enforcement for CMA | Requires a bounded reservation/settlement integration and lifecycle policy |

The existing `billing::webhook::provider_effective_balance` reads both
`wallet_balance_exact` and subscription `current_usage`, then subtracts accrued
usage. The reconcile worker uses that same function. Therefore direct CMA charges
on the same subscription can affect NyxID's reconciled balance without a NyxID
code change. Timing depends on provider aggregation and reconciliation; it is
not a real-time admission gate for CMA.

Direct export does not create per-Sandbox rows in NyxID's `/billing/usage`, does
not reserve credits before CMA allocates resources, and does not consume NyxID
grant/allowance benefits. Provider balance refreshes can produce aggregate
accounting adjustments; those are not per-Sandbox usage settlements. The existing
internal-ledger/provider drift checks iterate locally finalized metrics, so they
do not establish completeness for CMA-only provider metrics. Add an explicit
CMA ledger/provider comparison before paid activation.

## Recommended PR sequence

1. **This documentation PR:** review the price table, paused-disk policy,
   ownership and rollout requirements. No production configuration change.
2. **Zero-price provider validation:** configure only a controlled test
   subscription, using an approved existing credential reference. Exercise the
   corrected CMA lifecycle, retries and retained history. Compare exact ledger
   quantities with Lago current usage, the NyxID bill and a finalized test
   invoice. HTTP acceptance alone does not prove billing correctness.
3. **NyxID integration PR before a general prepaid launch:** add first-class CMA
   metric/display support and trusted, idempotent interval ingestion tied to the
   real owner/Sandbox; use NyxID's existing exact reservation and settlement
   machinery for benefits, prepaid limits and audit. Define bounded reservation
   windows, renewal failures, pause/storage policy and recovery explicitly.
   Coordinate CMA emission so each interval has exactly one billing path;
   direct Lago export and NyxID settlement must never both charge it.
4. **Approved paid activation:** confirm effective prices, funding behavior,
   spending limits and customer-facing disclosures before enabling nonzero
   charges. If a limited provider-direct pilot is chosen instead, explicitly
   accept its delayed balance visibility, absent per-Sandbox NyxID usage rows
   and lack of prepaid enforcement.

The integration PR is a separate implementation scope; this draft does not
promise that prepaid controls already exist. Billing stays disabled until the
selected path is verified.

## Activation boundary and history

Price approval must include an effective UTC timestamp. CMA has already retained
usage while billing was disabled, and its current exporter can drain outstanding
ledger rows when enabled. **Do not enable it at a paid rate and implicitly bill
all retained history.** First implement and verify a prospective billing cutoff
or explicitly handle pre-activation rows in the selected billing path, preserving
the ledger and immutable delivery receipts. Do not rewrite historical quantities.

Zero-price validation must also avoid repricing earlier validation usage when
nonzero prices are introduced. Use a controlled test subscription and verify
Lago's plan-version/subscription price-change behavior before scheduling paid
activation. Price changes need an effective version and a verified interval
boundary; a mutable plan price alone is not a historical pricing contract.

Existing Sandboxes and any observed legacy-writer gaps remain marked as partial
lifecycle coverage. No guessed historical intervals are billable. A successful
release must prove a fresh complete create → pause → resume → terminate lifecycle
after all writers have upgraded. Retained disk charges and usage-history retention
after resource deletion must be checked independently.

## References

- [CMA allocation usage contract](https://github.com/ChronoAIProject/cma/blob/0864e28ce6dee2081c3218aaf67df583e416031d/docs/CMA_Sandbox_Usage.md)
- [Lago setup](../LAGO_SETUP.md)
- [Exact credit accounting](../BILLING_EXACT_ACCOUNTING.md)
- [Billing operations and taxes](../BILLING_OPERATIONS.md)
- [NyxID provider balance calculation](../../backend/src/services/billing/webhook.rs)
- [NyxID reconciliation](../../backend/src/services/billing/reconcile.rs)
