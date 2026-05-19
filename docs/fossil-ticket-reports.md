# Fossil Ticket Reports

This repo uses two ticket reports in Fossil SCM.

## Reports

### Report #1: All Tickets

Shows every ticket with color-coded rows by status.

| Status | Color | Hex |
|--------|-------|-----|
| Open / Verified (Active) | Muted dark red | `#5c2a2a` |
| Review | Neutral dark gray | `#3a3a3a` |
| Fixed | Muted dark green | `#2a4a1a` |
| Tested | Muted dark teal | `#1a3a3a` |
| Deferred | Muted dark purple | `#2a2a4a` |
| Closed | Dark gray | `#2a2a2a` |

**Columns:** `#`, `mtime`, `type`, `status`, `subsystem`, `title`

**SQL:**
```sql
SELECT
  CASE WHEN status IN ('Open','Verified') THEN '#5c2a2a'
       WHEN status='Review' THEN '#3a3a3a'
       WHEN status='Fixed' THEN '#2a4a1a'
       WHEN status='Tested' THEN '#1a3a3a'
       WHEN status='Deferred' THEN '#2a2a4a'
       ELSE '#2a2a2a' END AS 'bgcolor',
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

The repo uses Fossil's `darkmode` skin. The original report colors were bright pastels (`#f2dcdc`, `#cfe8bd`, etc.) designed for light backgrounds. These were replaced with dark, desaturated tones that provide subtle row differentiation without eye-searing brightness.

When adding new statuses or changing colors, keep values dark and muted (R/G/B all below ~100) for dark mode compatibility.

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
