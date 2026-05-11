# NyxID Terms of Use — pre-commit fine-tooth-comb audit

**Status:** Detail audit of `TERMS_OF_USE_DRAFT.md` produced by specialist legal-documentation auditor. Companion to (not a replacement for) the prior section-by-section legal critique. Findings are observations and confirm-this items, not legal advice.

**Scope:** Placeholders, clarifications, internal consistency, factual claims pending verification, and quality nits. All emails / contacts / postal addresses / URLs / corporate identifiers currently in the draft are placeholders pending verification per the project owner.

---

## 1. Placeholder inventory

Every "filled-in-looking" identifier below is pending verification per the project owner. Comprehensive enumeration with line anchors:

| § | Current value in draft | Type | Owner to confirm | Note |
|---|---|---|---|---|
| L4 | `8 May 2026` | date | Legal | Last-updated date; author-picked, not legal-finalised |
| L8 | `ChronoAI Pte. Ltd.` | corporate ID | Legal | Final corporate form (Pte. Ltd. vs. Ltd. vs. Inc.) to be confirmed |
| §1.1 L14 | `ChronoAI Pte. Ltd.` | corporate ID | Legal | Second occurrence — must match L8 |
| §2 L63 | `ChronoAI Pte. Ltd., a company incorporated in Singapore` | corporate ID | Legal | Third occurrence in defined term — must match L8 / L14 |
| §2 L63 | `[Legal: confirm UEN]` | corporate ID | Legal | Explicit placeholder for Singapore UEN |
| §4.3 L146 | `privacy@nyxid.com` | email | Ops | GDPR Art. 22 contact address |
| §5.2 L183 | `security@nyxid.com` | email | Ops | Device-loss notification address |
| §6.5 L256 | `security@nyxid.com` | email | Ops | Incident reporting; second occurrence |
| §7.6 L282 | `privacy@nyxid.com` | email | Ops | US state privacy rights; second occurrence |
| §9.1 L335 | `ChronoAI Pte. Ltd.` | corporate ID | Legal | Fourth corporate-name occurrence (IP ownership) |
| §9.5 L357 | `legal@nyxid.com` | email | Ops | DMCA notice address; first occurrence |
| §14.4 L481 | `support@nyxid.com` | email | Ops | EU statutory withdrawal contact |
| §15.3 L503 | `legal@nyxid.com` | email | Ops | US arbitration opt-out address; second occurrence |
| §16.7 L541 | `legal@nyxid.com` | email | Ops | User-to-ChronoAI notice address; third occurrence |
| §16.9 L551 | `ChronoAI Pte. Ltd.` | corporate ID | Legal | Fifth corporate-name occurrence (contact block) |
| §16.9 L552 | `8 Marina Boulevard, #14-02, Singapore 018981` | address | Legal / Ops | Registered office; also flagged in Appendix A.1 #3 |
| §16.9 L553 | `legal@nyxid.com` | email | Ops | Fourth `legal@` occurrence |
| §16.9 L554 | `privacy@nyxid.com` | email | Ops | Third `privacy@` occurrence |
| §16.9 L555 | `security@nyxid.com` | email | Ops | Third `security@` occurrence |
| §16.9 L556 | `https://nyxid.io` | URL | Ops / Product | Canonical website; only occurrence in document |
| §1.5 L37 | `/privacy` | URL (relative) | Eng / Product | Privacy Policy route — not yet registered per Appendix A.4 |
| Appendix A.1 #1 L565 | `[Legal: confirm UEN]` | corporate ID | Legal | Self-reference to §2 placeholder |
| Appendix A.1 #3 L567 | `8 Marina Boulevard, #14-02, Singapore 018981` | address | Legal | Self-reference to §16.9 |
| Implicit | "ChronoAI" short name (used ~80+ times throughout) | corporate ID | Legal | If the final entity name changes (e.g., to "ChronoAI Labs Pte. Ltd."), every reference must be updated; not separately listed but flagged for global find-replace |
| §8.5 L304 | `Mailchimp` | sub-processor name | Ops / Eng | Named third-party; per Appendix A.3 verify still on sub-processor list |
| §8.5 L304 | `PostHog, US region` | sub-processor name + region | Ops / Eng | Named third-party + region claim; per Appendix A.3 verify |

**Totals:** 5 corporate-identity slots, 4 email mailboxes (`legal@`, `privacy@`, `security@`, `support@`), 1 website URL, 1 relative URL, 1 last-updated date, 1 postal address, 2 named sub-processors. Email occurrences total: `legal@nyxid.com` ×4 (L357, L503, L541, L553); `privacy@nyxid.com` ×3 (L146, L282, L554); `security@nyxid.com` ×3 (L183, L256, L555); `support@nyxid.com` ×1 (L481).

---

## 2. Clarifications needed

Open business / product / operational decisions that the draft presupposes but does not lock down:

1. **§14.1 / §14.4 / §14.5 — Will NyxID charge fees at v1 launch?**
   - Quote: "Where ChronoAI charges fees for access to all or part of the Services …"
   - Open question: Is the launch product free-tier-only, freemium, or paid-from-day-one? If free-only, §14.4 (refunds) and §14.5 (auto-renewal) are dead text and §10.3 bullet "failure to pay applicable service fees" is unreachable.
   - Owner: Product / Founders.
   - Impact: Determines whether §14 stays, gets pared down, or expands with pricing detail. Also affects §12.4 liability cap basis (USD 100 floor would always apply if no fees were ever paid).

2. **§1.2 — Does any Partner Application currently exist that triggers a Joint Controller arrangement?**
   - Quote: "Where NyxID utilises a shared or unified authentication system with any affiliated or related application ('**Partner Application**') …"
   - Open question: Is there a launch-day Partner Application (e.g., Aevatar)? If yes, the JCA essence must be disclosed concretely; if no, this section is forward-looking only.
   - Owner: Product.
   - Impact: Disclosure compliance under GDPR Art. 26; also affects Privacy Policy content.

3. **§4.1 — Which AI Features actually ship at v1?**
   - Quote: "(where enabled)" appears on Automated Risk Scoring, Anomaly Detection, Automated Approval Workflows.
   - Open question: At launch, are any of risk-scoring / anomaly-detection / automated-approval features live? "Where enabled" is a hedge, but disclosure for unbuilt features creates a misleading impression of capability.
   - Owner: Eng / Product.
   - Impact: §4.6 EU AI Act conformity (Appendix A.2 #4) cannot be assessed until this is known.

4. **§3.4 — Is there a separate Developer Agreement, or is §3.4 the entire developer contract?**
   - Quote: "If you are a developer integrating a Partner Application with NyxID …"
   - Open question: Does ChronoAI plan to publish a stand-alone Developer Terms / API Terms, or is §3.4 it? Also relates to Appendix A.2 #10 (SDK licence model).
   - Owner: Legal / Product.
   - Impact: Determines whether §3.4 should be expanded with rate limits, SLA, brand-use specifics, or relocated.

5. **§7.6 — Universal Opt-Out Mechanism (UOOM / GPC) — does the live App honour these signals today?**
   - Quote: "ChronoAI honours Universal Opt-Out Mechanism (UOOM) signals (including Global Privacy Control) where required by applicable state law."
   - Open question: Is GPC detection implemented in the web App today, or is this aspirational?
   - Owner: Eng.
   - Impact: Misalignment = misrepresentation. California / Colorado enforcement risk.

6. **§7.4 — What is the actual list of countries where personal data is stored / processed?**
   - Quote: "may be transferred to, stored in, and processed in countries outside your jurisdiction, including Singapore and other countries"
   - Open question: Which exact regions does the production stack run in? "Other countries where ChronoAI's service providers operate" is vague. Privacy Policy says the detail lives there — but it must be true and concrete.
   - Owner: Eng / Ops.
   - Impact: SCCs choice depends on actual transfer destinations.

7. **§8.6 — Is the iOS app actually being distributed via the Apple App Store at launch?**
   - Quote: "The App is distributed through the Apple App Store and the Google Play Store"
   - Open question: Is the mobile app live in both stores, or only one, or still in beta / TestFlight at the ToS effective date?
   - Owner: Product / Ops.
   - Impact: §8.6's Apple-specific clauses (Licensed Application, third-party beneficiary, etc.) become inert if iOS isn't shipped. Also affects §14.2 "Apple In-App Purchase" reference.

8. **§10.3 bullet "death of the User" — what is the operational process?**
   - Quote: "death of the User (upon notification by a verified next of kin or legal representative)"
   - Open question: Is there a documented verification workflow? Does NyxID have an inheritance / deceased-user policy? Without one, the clause may be unenforceable as written.
   - Owner: Ops / Legal.
   - Impact: Operational only at v1, but should be backed by a SOP.

9. **§14.5 California ARL — does the live billing flow show pre-renewal reminders today?**
   - Quote: "ChronoAI will provide reminders of upcoming renewals where required by applicable law"
   - Open question: Is reminder-email logic implemented? Tied to Appendix A.2 #12 and A.3 §14.5.
   - Owner: Eng / Product.
   - Impact: Cannot claim ARL compliance in document if implementation gap exists.

10. **§15.2 mediation pre-condition — is a 14-day mediation window appropriate for an early-stage SaaS?**
    - Quote: "If the dispute is not settled by mediation within fourteen (14) days of commencement, it shall be referred to and finally resolved by arbitration under the SIAC Rules."
    - Open question: Many SaaS ToS skip mandatory mediation. Is this what ChronoAI wants operationally (cost, delay), or a placeholder?
    - Owner: Legal / Founders.
    - Impact: Procedural — affects all SG-jurisdiction disputes.

11. **§15.3 — Texas as US choice-of-law: business reason?**
    - Quote: "this Agreement shall be governed by the laws of the State of Texas"
    - Open question: Why Texas (versus Delaware, the SaaS default)? Does ChronoAI have a Texas nexus? If not, choice may be challenged under California §1646.5 etc.
    - Owner: Legal / Founders.
    - Impact: US jurisdiction strategy.

12. **§15.4 — UK arbitration via LCIA: appropriate for consumer disputes?**
    - Quote: "the dispute shall be finally resolved under the LCIA Rules. The seat of arbitration shall be London."
    - Open question: LCIA fees are high; for consumer disputes < £10k this could be unconscionable under CRA 2015. Should the UK clause carve out a small-claims threshold parallel to §15.1's "small claims court" carve-out?
    - Owner: Legal.
    - Impact: Enforceability of UK arbitration clause.

13. **§15.3 30-day opt-out — how is the "first acceptance" date recorded?**
    - Quote: "within thirty (30) days of first accepting these Terms"
    - Open question: Does the registration flow currently record consent timestamp on a per-user record, server-side, with version of ToS? Appendix A.4 #3 acknowledges the consent record isn't built yet.
    - Owner: Eng.
    - Impact: Unable to compute opt-out window without server-side consent record.

14. **§9.4 — "ChronoAI will not use your User Content to train AI models … without your separate, informed consent" — does any current process use User Content for training?**
    - Quote: "ChronoAI will not use your User Content to train AI models for resale or external distribution without your separate, informed consent."
    - Open question: Is any feedback / bug-report content currently fed to model fine-tuning? Where is the consent capture UX?
    - Owner: Eng / Product.
    - Impact: Misrepresentation risk if any pipeline exists.

15. **§7.6 list of US states — is the list intended to be exhaustive of "as-of effective date" or evergreen?**
    - Quote: "California, Virginia, Colorado, Connecticut, Utah, Texas, Oregon, Montana, Iowa, Tennessee, Delaware, New Jersey, New Hampshire, Maryland, Minnesota …"
    - Open question: States enact new privacy laws regularly. Is the maintenance plan a re-issue on each state addition, or does "another U.S. state with a comprehensive privacy law in force" do the work? Currently both phrasings co-exist.
    - Owner: Legal.
    - Impact: Drift risk; needs maintenance SOP.

16. **§3.2 Local Agent — does on-device storage actually never transmit credentials to ChronoAI?**
    - Quote: "credentials to remain on your device and never be transmitted to ChronoAI's servers"
    - Open question: Per CLAUDE.md §6 / §8, node agent credentials are encrypted locally; SHA-256 hashes are stored server-side. But the WebSocket connection itself reaches ChronoAI's WS endpoint — confirm the wording "never be transmitted" can't be misread to include the hash / signing-secret hash material that is server-side.
    - Owner: Eng / Legal.
    - Impact: Wording precision — could be construed as misleading.

17. **§8.5 — "waitlist sign-up data … is not stored persistently by NyxID" — is this current behaviour?**
    - Quote: "waitlist sign-up data (first name, email, optional company name) may be transmitted to third-party mailing list providers (such as Mailchimp) for communications purposes, and is not stored persistently by NyxID."
    - Open question: Is the waitlist form actually wired to forward-only-to-Mailchimp with no DB write? Or is there a `waitlist` collection? Tied to A.3 §8.5.
    - Owner: Eng.
    - Impact: Privacy Policy congruence + truth of representation.

18. **§16.7 — Does ChronoAI commit to in-App notification capability as a primary notice channel?**
    - Quote: "Notices … will be provided by posting to the App, through in-app notifications, or via the email address associated with your account."
    - Open question: Does the App support targeted in-app notifications today? If not, the clause leans heavily on email-only.
    - Owner: Eng / Product.
    - Impact: Effective-notice doctrine in dispute scenarios.

---

## 3. Internal-consistency findings

### 3.1 Cross-reference accuracy

- **§10.4 L393** — surviving-sections list "(including Sections 3.4, 5, 6, 7, 9, 11, 12, 13, 14, 15, and 16)". Missing: §4 (AI Disclosures), §8 (Third-Party Integrations), §2 (Definitions). Consider whether §4 disclosures to past Users should survive.
- All other cross-references (§1.7 → §8.7, §5.1 → §4.3, §9.3 → §9.2, §8.6 → §16.9, §16.7 → §16.9, §2 LicensedApplication → §8.6, §2 NyxIDContent → §9.1) verified accurate.

### 3.2 IMPORTANT NOTICE banner vs. body heading mismatches

- **§9** body heading is "INTELLECTUAL PROPERTY RIGHTS" — banner says "INTELLECTUAL PROPERTY AND USER-GENERATED CONTENT LICENCE." Heading should be widened to match the banner, or banner narrowed.
- **§10** body heading is "SERVICE CHANGES, SUSPENSION AND TERMINATION" — banner says "TERMINATION RIGHTS." Acceptable abbreviation.
- **§14** body heading is "FEES AND PAYMENT" — banner says "AUTOMATIC RENEWAL AND REFUNDS." §14 body covers both; consider renaming heading to "FEES, PAYMENT, REFUNDS AND AUTO-RENEWAL."
- **§15** banner uses hyphenated `CLASS-ACTION`; body §15.5 uses open `CLASS ACTION`. Pick one.

### 3.3 Defined-term hygiene

- **"Alert"** (defined §2 L59) — **never used in body**. Either delete or add a referring use.
- **"Intellectual Property"** (defined §2 L68) — underused; §9 body uses the phrase lowercase. Either capitalise body uses or drop the §2 definition.
- **"Partner Application"** — defined in both §2 L76 AND in-line at §1.2 L20. §2 definition is narrower than the §1.2 in-line. Reconcile.
- **"Agreement"** — bold-defined twice: preamble L8 AND §1.1 L14. Duplicate.
- **"Terms"** — bold-defined twice: preamble L8 AND §1.1 L14. Duplicate.
- **"User / you / your"** — bold-defined three times: preamble L8, §1.1 L14, §2 L83. Triplicate.
- **"ChronoAI"** — bold-defined three times: preamble L8, §1.1 L14, §2 L63. Triplicate; §2 should be canonical (it has the UEN slot).
- **"PDPA"** — bold-defined in §2 L77 AND in-line at §7.2 L266. Duplicate.
- **"Services"** — bold-defined in §2 L81 AND in-line at §1.1 L14. Duplicate.
- **"Service Fees"** (§14.1 L469) — bold-defined in §14 only; not in §2. Either promote to §2 or accept the convention.
- **"OAuth Providers"** (§8.2 L292), **"App Platforms"** (§8.6 L308), **"Third-Party Services"** (§8.1 L288) — bold-defined inline but not in §2. Promote for consistency.
- **"CCPA/CPRA"** (§7.6 L282), **"Apple"** (§8.6 L312), **"External Services"** (§8.6 L319), **"Google"** (§8.6 L323), **"ATT"** (§8.6 L308), **"Claim"** (§15.1 L493), **"SIAC"** (§15.2 L497) — bold-defined inline; acceptable as section-scoped terms.

### 3.4 Capitalisation drift

- **"MFA Secrets" (§2 L72)** vs. **"MFA secrets"** (§5.3 L191). Same defined term; align.
- **"SSH Certificate" (§2 L82)** vs. **"SSH certificates"** lowercase (§3.2 L99, §6.2 L238). Align.
- **"Personal Data" (§2 L78)** is essentially abandoned in body, which uses **"personal data"** throughout §7. Either consistently use the defined term or drop the definition.
- **"OAuth Provider(s)"** — §6.3 L247 uses lowercase "third-party OAuth providers"; §8.2 L292 introduces capitalised "OAuth Providers." Inconsistent.
- **"Approval Request"** (§2 L62) vs. **"approval workflow"** (§6.2 L237) vs. **"Approval Request workflow"** (§12.3 L436). Consider whether "Approval Request workflow" should be separately defined.

### 3.5 Numbering integrity

- All §§1–16 sub-numbering sequential with no gaps or duplicates.
- §13 is flat-list with no sub-headings, unlike adjacent §12 and §14. Acceptable but inconsistent.
- Appendix A.1 + A.2 use numbered list (items 1–17 crossing subsection boundary); A.3 and A.4 use bullets. Inconsistent — either number all four or document the distinction.

### 3.6 Appendix A coherence

- **A.2 #9 (Indemnity)** targets §5.6, §13, §3.4 — but §13 contains representations + warranties, NOT indemnity language (the indemnity is in §5.6). Either revise A.2 #9 to "§3.4 and §5.6" or add indemnity language to §13.
- All other A.1/A.2/A.3 cross-references to §-targets verified accurate.

### 3.7 SIAC terminology (§15.2)

- "Chairman of SIAC" is outdated. Under SIAC Rules 2016/2025 the appointing authority is "the President of the Court of Arbitration of SIAC." Update.

---

## 4. Factual claims requiring engineering / ops verification

Consolidated register. Items marked **[NEW]** were introduced by edits after the previous review.

| § | Claim (one-sentence) | Verification owner | What evidence would close it |
|---|---|---|---|
| §3.2 L95 | Proxied request/response bodies are buffered in memory only and not written to disk or persistently logged. | Eng | Source-code audit of `proxy_service::execute_proxy` and node WS handler confirming no `tokio::fs` writes or persistent log sinks for body bytes. |
| §3.2 L100 / §4.2 L142 | Credential inputs to AI chat assistant are encrypted locally and not transmitted to LLM. | Eng | Audit of frontend / chat-pipeline code showing client-side credential redaction or encryption before any LLM API call; verify scope of "local." |
| §3.4 L115 **[NEW]** | "ChronoAI may inspect aggregate API usage records for abuse detection." | Eng / Ops | Confirm whether aggregate usage inspection is in production today; whether logs are sufficient; whether retention is documented. |
| §4.1 L127–L129 | Automated Risk Scoring, Anomaly Detection, Automated Approval Workflows are AI features "where enabled." | Eng / Product | Confirm whether any of these are shipped at the ToS effective date. |
| §4.6 L158 | "ChronoAI has classified its AI systems and applied appropriate risk management measures." | Legal / Eng | Internal AI risk classification record per EU AI Act Annex III analysis. |
| §5.1 L174 | Users may "export your data in a portable format where technically feasible." | Eng / Product | Confirm `/export` endpoint or in-App export UI exists. |
| §6.4 L252 | "ChronoAI does not store biometric templates." | Eng | Audit mobile app for biometric API surfaces; confirm no server-side biometric processing. |
| §7.5 L278 | AES-256 at rest, TLS 1.2+ in transit, audit logging of all credential access events. | Eng / Ops | TLS config audit; AES-256 confirmed in `crypto/aes.rs`; audit-log coverage report. |
| §7.6 L282 **[NEW]** | "ChronoAI honours UOOM / GPC signals." | Eng | Confirm web App detects `Sec-GPC: 1` header / `navigator.globalPrivacyControl`. |
| §7.6 L282 | "Does not 'sell' for monetary or other consideration; does not 'share' for cross-context behavioural advertising." | Ops / Legal | Confirm no advertising-attribution SDK, no advertising-network beacon. |
| §8.3 L296 | Channel Platforms listed (Telegram, Lark/Feishu, Discord, OpenClaw). | Eng | Cross-check `services/channel_adapters/` — confirm each named platform has a live adapter. |
| §8.5 L304 | Waitlist data → Mailchimp; usage telemetry → PostHog (US region), opt-in only. | Eng / Ops | Confirm Mailchimp + PostHog US region; confirm telemetry opt-in gating. |
| §8.5 L304 | "Waitlist data is not stored persistently by NyxID." | Eng | Confirm no `waitlist` MongoDB collection; form action direct-to-Mailchimp. |
| §8.6 L308 | ATT consent prompt wired in iOS app for any cross-app tracking. | Mobile Eng | Confirm ATT dialog present if any tracking SDK exists. |
| §9.1 L335 | "Components of the App made available under open-source licenses are licensed separately." | Legal / Eng | Confirm which components are OSS-licensed; confirm OSI-approved licence files present. |
| §9.4 L353 **[NEW]** | "ChronoAI will not use User Content to train AI models for resale or external distribution without separate consent." | Eng / Product | Confirm no current pipeline feeds User Content into model fine-tuning. |
| §11.1 L399 | "ChronoAI maintains documented incident response procedures." | Ops / Eng | Internal IR runbook; tabletop exercise record. |
| §11.2 L405 **[NEW]** | "No later than 3 calendar days of assessing the breach as notifiable under the Singapore PDPA." | Legal | Confirm phrasing tracks PDPA Section 26D + PDPC guidance. |
| §14.5 L485 **[NEW]** | "Renewal reminders provided where required by applicable law (incl. CA ARL)." | Eng / Product | Confirm billing system supports 3–21 day pre-renewal reminders; UI shows ARL-compliant disclosures. |
| §15.3 L503 **[NEW]** | "Within thirty (30) days of first accepting these Terms" — implies recorded acceptance date. | Eng | Confirm registration flow records ToS version + acceptance timestamp server-side. |
| §6.1 L229 | "Security practices informed by ISO/IEC 27001, NIST CSF, SOC 2." (disclaimed as non-certification) | Ops | Confirm internal alignment documents; ensure marketing copy doesn't overstate. |
| §8.6 L323 | Compliance with Google Play Developer Distribution Agreement + Program Policies. | Mobile Eng / Ops | Confirm Android app data-safety form complete and matches the document. |

**Total:** 22 factual claims pending verification, of which 6 are NEW since the prior review pass.

---

## 5. Quality nits

### 5.1 Spelling: British vs. American drift

Document uses British English overall. Drifts to fix:

- **§5.5 L210** — "applicable open-source **license** terms" → `licence`
- **§9.1 L335** — "open-source **licenses**" (×2 in same sentence) → `licences`
- **§9.3 L345** — "applicable open-source **license** terms" → `licence`
- **§9.5 L357** — DMCA body uses "**license**" generally; align to `licence` outside the proper noun "DMCA"
- **§3.3 L104 / §16.6 L537 / §10.1 L363** — inconsistent "endeavours" vs. "efforts" — standardise to one form
- DMCA is American by definition; the proper noun is untouchable but adjacent prose can use British spelling

### 5.2 Bold-for-emphasis breaking convention

- **§11.2 L405** — `**no later than 3 calendar days**` is bold for emphasis; document elsewhere reserves bold for defined terms and contact addresses. Unbold or restructure.

### 5.3 List-style inconsistency

- **§8.6** — iOS bullet list uses 10 distinct bullets; Android paragraph uses single in-line `(i)–(iv)` numbered list. Either format both as bullets or both as in-line.
- **Appendix A** — A.1 and A.2 numbered, A.3 and A.4 bulleted. Either number all four or document the distinction.

### 5.4 Heading-style inconsistency

- **§9 / §10 / §14 / §15 banner-vs-heading** — see §3.2 above. Banner promises more than heading shows.

### 5.5 Specific phrasings

- **L8 preamble** — "interact" repeated in same sentence; vary.
- **§1.4 L33** — references "the 'Last Updated' date" (capital U) but document header at L4 uses "Last updated" (lowercase u). Align.
- **§1.6 L43** — long parenthetical sub-list (a)(b)(c) inside a single bullet; restructure as nested list.
- **§4.1 L131** — "not every feature above may be active" → "not all features listed above are necessarily active."
- **§7.6 L282** — list of US states ends with em-dash followed by catch-all "another U.S. state with a comprehensive privacy law in force at the time of your request" — phrasing dual-tracked; pick one of (enumeration-only) or (catch-all-only). Currently both phrasings co-exist.
- **§10.4 L393** — surviving-sections annotations mixed: some sections have parenthetical titles, others don't.
- **§12.5 L447** — "LIMIT OR EXCLUDE ANY LIABILITY THAT CANNOT BE EXCLUDED OR LIMITED" — verb pair inverted. Suggest "EXCLUDE OR LIMIT ANY LIABILITY THAT CANNOT LAWFULLY BE EXCLUDED OR LIMITED."
- **§15.2 L497** — "Chairman of SIAC" — outdated; should be "President of the Court of Arbitration of SIAC."
- **§16.7 L541** — preposition inconsistency: "by posting to the App, through in-app notifications, or via the email address." Parallel "by … by … by …" or "via … via … via …".

### 5.6 Cross-section overlaps

- **§13 vs. §5.4** — both cover "use the App only for lawful purposes." Consider deduplicating.
- **§14.2** — references "Apple In-App Purchase or Google Play Billing" — only relevant if NyxID actually sells anything via App Platform IAP (see Clarifications #1, #7).
- **§15.5 "court of competent jurisdiction"** — if §15.2/§15.3/§15.4 choice-of-law applies, the court would presumably follow the chosen forum. Confirm intent.

---

## Net summary

- **26 placeholder rows** (5 corporate-name + 1 explicit `[UEN]` + 11 email occurrences across 4 mailboxes + 1 URL + 1 relative URL + 1 date + 1 address + 2 sub-processor names)
- **18 clarification items** requiring product / business / legal decisions before publication
- **~50 internal-consistency observations** across cross-references, defined-term hygiene (6 duplicate definitions, 2 unused/underused terms, 4 inline-defined-but-not-in-§2 terms), capitalisation drift (4 defined terms not consistently capitalised in body), banner-vs-heading mismatches (§9, §10, §14), and SIAC terminology
- **22 factual claims** pending engineering / ops verification, of which 6 are NEW since the prior review
- **30+ quality nits** including 5–6 American-spelling drifts on `license/licence`, `endeavours`/`efforts` inconsistency, bold-for-emphasis breaking convention in §11.2, list-style inconsistencies in §8.6 and Appendix A, and SIAC "Chairman" terminology
