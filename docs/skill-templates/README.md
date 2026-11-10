# NYX skill template examples

Reusable example skills and one skillset written in the
[NYX skill template](../SKILL_TEMPLATES.md). They live here, not under
`skills/`, so `nyxid ai-setup` does not install them; publish them to Ornn
deliberately.

| Kind | Skills |
| --- | --- |
| Capability (one provider) | `sms-send-twilio`, `sms-send-telnyx`, `payment-link-stripe`, `phone-number-sg-twilio`, `phone-number-sg-telnyx`, `voice-agent-telnyx`, `voice-agent-elevenlabs`, `records-notion`, `records-google-sheets`, `docs-google-docs`, `files-google-drive` |
| Behaviour (any provider) | `phone-conversation`, `phone-call-closing`, `caller-details-readback`, `confirm-from-tool-result`, `answer-from-knowledge`, `human-follow-up` |
| Workflow (any business) | `booking-with-availability`, `order-with-payment-link` |
| Skillset (one use case) | `phone-front-desk-sg`: 14 members, master prompt in `SKILLSET.md` |

Capability skills name one provider and no business. Workflow and behaviour
skills name neither. A skillset picks the members for one use case, and its
master prompt, written in the template, says how they combine and states the use
case it is for. Business facts (hours, rules, FAQ, catalog) belong in a separate
facts pack, never in these skills.

`SKILLSET.md` is a local stand-in for an Ornn skillset: its frontmatter holds
the skillset metadata and `members`, and its body is the master prompt
(`instructions`, at most 8,000 characters).

## Check them

```bash
NYX_SKILL_TEMPLATE_DIR=docs/skill-templates/examples \
NYX_SKILL_TEMPLATE_REPORT=/tmp/skill-template-report.json \
RUST_MIN_STACK=8388608 \
cargo test -p nyxid --bin nyxid-server validate_skill_template_dir -- --ignored --nocapture
```

The check parses every `SKILL.md` exactly as NyxID parses a pinned package,
presents each skillset as one skill whose dependencies are its members, reports
missing members, cycles and closure sizes, and writes each outline plus the
exact `preview_agent_skill` results to the report.

## Preview the cards

```bash
cp /tmp/skill-template-report.json frontend/public/skill-template-report.json
cd frontend && npx vite --port 3000
# open http://localhost:3000/skill-template-preview.html
```

The local preview renders each skill card with `SkillTemplateCard` next to its
tool results, in dark and light themes. The copied report is a local file; do
not commit it.
