---
name: records-google-sheets
description: "Keep simple business records in a Google Sheet through NyxID: read rows, append new rows and update cells, confirming saves from the Sheets response."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - sheets_get_values
    - sheets_append_values
    - sheets_update_values
    - sheets_create_spreadsheet
  tag:
    - records
    - google-sheets
    - capability
---

# Records in Google Sheets

## Overview
I keep simple records in a Google Sheet. I read rows, add new ones at the end
and update cells, and I only say something is saved when Sheets confirms it.

It suits a shared sheet the team already watches. It is not for many writers at
once or for calculations you need exact.

It needs a Google Workspace connection in NyxID with access to the sheet.

## Approach
1. **Read the header.** I read the first row so every value lands in the right column.
2. **Look before writing.** I search existing rows to avoid duplicates.
3. **Append or update.** New records go at the end; changes go to the exact row.
4. **Confirm from Sheets.** Saved only when the response reports the updated range.

## Needs and produces
Needs: a Google Workspace connection in NyxID with access to the spreadsheet.
Produces: an appended or updated row, with the range Sheets reports.

## Details
- Read with `sheets_get_values` on a bounded range (for example `Bookings!A1:O200`).
- Append with `sheets_append_values`, `valueInputOption=USER_ENTERED`, `insertDataOption=INSERT_ROWS`.
- Update one row with `sheets_update_values` on its exact A1 range.
- Saved means `updates.updatedRows` (append) or `updatedCells` (update) is at least 1.
- Sheets has no row ids: keep a reference column you write yourself.
