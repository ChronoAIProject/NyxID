---
name: records-notion
description: "Keep business records in Notion databases through NyxID: look up, create and update rows, and confirm a save only from Notion's own response."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - notion_retrieve_database
    - notion_query_database
    - notion_create_page
    - notion_update_page
  tag:
    - records
    - notion
    - capability
---

# Records in Notion

## Overview
I keep records such as bookings, orders and follow-ups in Notion databases. I
look rows up, add new ones and update them, and I only say something is saved
when Notion returns the saved page.

It suits small teams who already work in Notion. It is not for large data or
reporting.

It needs a Notion connection in NyxID shared with the databases. Each save adds
or changes one row.

## Approach
1. **Learn the database.** I read its properties and types before writing anything.
2. **Look before writing.** I search for an existing row first, so I update instead of duplicating.
3. **Save the row.** I create or update exactly the fields that changed.
4. **Confirm from Notion.** I treat it as saved only when Notion returns the page and its id.

## Needs and produces
Needs: a Notion connection in NyxID with access to the database.
Produces: a created or updated Notion page, with its id and any formula values.

## Details
- `notion_retrieve_database` gives property names and types; match them exactly (title, rich_text, number, select, date, phone_number).
- Look up with `notion_query_database` and a `filter` on a stable property (reference, phone, date). Never claim "found" without a returned row.
- Create with `notion_create_page` (`parent.database_id`, `properties`); update with `notion_update_page`.
- Saved means the response has `"object": "page"` and an `"id"`. Formula and rollup values such as totals and codes come back on that page: read them from there.
- A validation error means nothing was saved: fix the body once, and never repeat an identical failed call.
