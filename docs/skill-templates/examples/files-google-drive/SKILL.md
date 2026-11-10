---
name: files-google-drive
description: "Store, copy and find files in Google Drive through NyxID, for receipts, exports and shared templates."
version: "1.0"
license: MIT
metadata:
  category: tool-based
  tool-list:
    - drive_list_files
    - drive_upload_file
    - drive_copy_file
    - drive_export_file
    - drive_get_file
  tag:
    - files
    - google-drive
    - capability
---

# Files in Google Drive

## Overview
I store, copy and find files in Google Drive, such as receipts, exports and
shared templates.

It suits keeping files where the team already looks. It is not a backup system.

It needs a Google Workspace connection in NyxID with access to the folder.

## Approach
1. **Find the folder.** I look up the folder you name before saving anything.
2. **Save or copy the file.** Upload a new file, or copy a template to start from.
3. **Confirm and share the link.** I read the file back and give you its link.

## Needs and produces
Needs: a Google Workspace connection in NyxID with access to the folder.
Produces: a Drive file id and link.

## Details
- Find with `drive_list_files` and a `q` filter (`name = '...' and '<folder id>' in parents`).
- Upload with `drive_upload_file`; copy a template with `drive_copy_file`; export Docs and Sheets as PDF with `drive_export_file`.
- Confirm with `drive_get_file`; never change sharing settings unless the owner asks.
