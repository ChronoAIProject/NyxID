---
name: docs-google-docs
description: "Create and update Google Docs through NyxID, for written records such as summaries, notes and confirmations."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - docs_create_document
    - docs_get_document
    - docs_batch_update_document
  tag:
    - documents
    - google-docs
    - capability
---

# Documents in Google Docs

## Overview
I create a Google Doc or add to an existing one, for written records such as
call summaries, notes or confirmations.

It suits records people read rather than filter. For rows of data, use a sheet
or a database instead.

It needs a Google Workspace connection in NyxID.

## Approach
1. **Find or create the doc.** I reuse the doc you name, or create a new one with a clear title.
2. **Write the content.** Plain headings and short paragraphs.
3. **Confirm it saved.** I read the document back and give you its link.

## Needs and produces
Needs: a Google Workspace connection in NyxID.
Produces: a Google Doc id and link with the new content.

## Details
- Create with `docs_create_document` (`title`); add text with `docs_batch_update_document` `insertText` requests at the end of the body.
- Confirm with `docs_get_document`; the link is `https://docs.google.com/document/d/<id>`.
- Never paste secrets or payment details into a doc.
