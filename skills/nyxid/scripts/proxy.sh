#!/usr/bin/env bash
# Deprecated: this helper script was removed when the NyxID skill migrated to
# the Anthropic Agent Skills spec. Use the `nyxid` CLI directly:
#   nyxid proxy request <slug> <path> -m <METHOD> [-d <body>]
#
# This stub is preserved only so older `nyxid` CLI binaries can complete their
# `nyxid update` flow without a 404. After your first `nyxid update` with a
# CLI from this release or later, this file will be removed automatically.
echo "[deprecated] proxy.sh is no longer part of the NyxID skill. Run 'nyxid proxy request <slug> <path> -m <METHOD> [-d <body>]' instead." >&2
exit 1
