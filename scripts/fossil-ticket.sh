#!/usr/bin/env bash
set -euo pipefail

# Fossil ticket query tool.
# Fetches ticket metadata and comments from a local Fossil checkout
# using a UUID prefix (as copied from the web UI).
#
# Usage: fossil-ticket.sh <uuid-prefix>

if [[ $# -ne 1 ]]; then
    echo "Usage: ${0##*/} <uuid-prefix>" >&2
    exit 1
fi

PREFIX="$1"

if [[ ! "$PREFIX" =~ ^[0-9a-fA-F]+$ ]]; then
    echo "Error: UUID prefix must be a hex string, got: $PREFIX" >&2
    exit 1
fi

# --- Query 1: Resolve prefix to exactly one ticket ---

TICKET_JSON=$(echo -e ".mode json\nSELECT tkt_uuid, title, status FROM TICKET WHERE tkt_uuid LIKE '${PREFIX}%';" | fossil sql --readonly)

ROW_COUNT=$(echo "$TICKET_JSON" | jq 'length')

if [[ "$ROW_COUNT" -eq 0 ]]; then
    echo "Error: no ticket found matching prefix: $PREFIX" >&2
    exit 1
fi

if [[ "$ROW_COUNT" -gt 1 ]]; then
    echo "Error: prefix '$PREFIX' matches multiple tickets:" >&2
    echo "$TICKET_JSON" | jq -r '.[] | "  \(.tkt_uuid)  \(.title)"' >&2
    exit 1
fi

UUID=$(echo "$TICKET_JSON" | jq -r '.[0].tkt_uuid')
TITLE=$(echo "$TICKET_JSON" | jq -r '.[0].title')
STATUS=$(echo "$TICKET_JSON" | jq -r '.[0].status')

# --- Query 2: Fetch comments ---

COMMENTS_JSON=$(echo -e ".mode json\nSELECT datetime(C.tkt_mtime) as time, C.login, C.icomment as comment FROM TICKETCHNG AS C JOIN TICKET AS T ON C.tkt_id = T.tkt_id WHERE T.tkt_uuid LIKE '${PREFIX}%' AND C.icomment IS NOT NULL ORDER BY C.tkt_mtime ASC;" | fossil sql --readonly)

# --- Print output ---

echo "Title: $TITLE"
echo "Status: $STATUS"
echo "UUID: $UUID"
echo ""

COMMENT_COUNT=$(echo "$COMMENTS_JSON" | jq 'length')

if [[ "$COMMENT_COUNT" -eq 0 ]]; then
    echo "No comments."
else
    echo "--- Comments ---"
    echo ""
    echo "$COMMENTS_JSON" | jq -r '.[] | "[\(.time)] \(.login):\n\(.comment)\n"'
fi
