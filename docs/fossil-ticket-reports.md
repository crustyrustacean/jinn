# Fossil Ticket Reports

This repo uses two ticket reports in Fossil SCM.

## Reports

### Report #1: All Tickets

Shows every ticket with color-coded rows by status.

| Status | Color | Hex |
|--------|-------|-----|
| Open / Verified (Active) | Soft salmon/pink | `#f5c6c6` |
| Review | Warm gray | `#d8d8d8` |
| Fixed | Sage green | `#c8e6b8` |
| Tested | Soft teal | `#b8d8d8` |
| Deferred | Lavender | `#d0c8e8` |
| Closed | Cool gray | `#c8c8c8` |

**Columns:** `#`, `mtime`, `type`, `status`, `subsystem`, `title`

**SQL:**
```sql
SELECT
  CASE WHEN status IN ('Open','Verified') THEN '#f5c6c6'
       WHEN status='Review' THEN '#d8d8d8'
       WHEN status='Fixed' THEN '#c8e6b8'
       WHEN status='Tested' THEN '#b8d8d8'
       WHEN status='Deferred' THEN '#d0c8e8'
       ELSE '#c8c8c8' END AS 'bgcolor',
  substr(tkt_uuid,1,10) AS '#',
  datetime(tkt_mtime) AS 'mtime',
  type,
  status,
  subsystem,
  title
FROM ticket
```

### Report #2: Open Tickets

Shows only tickets with `status='Open'`. No status column (redundant) and no background colors.

**Columns:** `#`, `mtime`, `type`, `subsystem`, `title`

**SQL:**
```sql
SELECT substr(tkt_uuid,1,10) AS '#', datetime(tkt_mtime) AS 'mtime', type, subsystem, title
FROM ticket WHERE status='Open'
```

## Color Scheme Rationale

The repo uses Fossil's `darkmode` skin, which **forces black text** (`color: black`) on report table cells (see `body.report table.report tr td { color: black }` in the CSS). This means background colors **must** be light enough for black text to be readable.

The current colors are muted pastels — light enough for black text but desaturated enough to not be eye-searing on the dark page background. Each color uses a distinct hue for easy visual differentiation.

When changing colors, ensure:
- Lightness is high enough for black text (keep R/G/B values above ~180)
- Colors are distinct from each other (different hue families)
- Saturation is moderate (not neon, not gray)

## Ticket Schema

These are the valid values for each ticket field (defined in `ticket-common` config):

| Field | Values |
|-------|--------|
| type | `Code_Defect`, `Build_Problem`, `Documentation`, `Feature_Request`, `Incident` |
| priority | `Immediate`, `High`, `Medium`, `Low`, `Zero` |
| severity | `Critical`, `Severe`, `Important`, `Minor`, `Cosmetic` |
| resolution | `Open`, `Fixed`, `Rejected`, `Workaround`, `Unable_To_Reproduce`, `Works_As_Designed`, `External_Bug`, `Not_A_Bug`, `Duplicate`, `Overcome_By_Events`, `Drive_By_Patch`, `Misconfiguration` |
| status | `Open`, `Verified`, `Review`, `Deferred`, `Fixed`, `Tested`, `Closed` |
| subsystem | `Other`, `Workflows`, `Chat`, `UI_UX`, `Tools` |

## How to Modify Reports

### Via Web UI (recommended for interactive use)

1. Go to `/ticket` → click the report name → click "Edit" on the report page
2. Or go directly to `/rptedit?rn=N` where N is the report number
3. To create a new report: `/rptnew`

### Via CLI (for agents / automation)

Reports are stored in the `reportfmt` table in the Fossil repository database. Modify using `fossil sql`:

```bash
# List all reports
fossil sql "SELECT rn, title FROM reportfmt;"

# View a report's SQL
fossil sql "SELECT sqlcode FROM reportfmt WHERE rn=1;"

# Update a report's SQL (use double-single-quotes for literals)
fossil sql "UPDATE reportfmt SET sqlcode='...' WHERE rn=1;"

# Update color key (cols field)
fossil sql "UPDATE reportfmt SET cols='...' WHERE rn=1;"

# Add a new report
fossil sql "INSERT INTO reportfmt (owner, title, cols, sqlcode, jx) VALUES (NULL, 'Report Name', '', 'SELECT ...', '{}');"
```

**Important:** SQL string literals in Fossil SQL require double-single-quotes (`''`) for embedded quotes.

### Via Config Import/Export

```bash
# Export all config (includes reports)
fossil configuration export all /tmp/config.txt

# Export just ticket config
fossil configuration export ticket /tmp/ticket-config.txt

# Import after editing
fossil configuration import /tmp/ticket-config.txt
```
