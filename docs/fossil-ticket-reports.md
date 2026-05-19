# Fossil Ticket Reports

This repo uses two ticket reports in Fossil SCM.

## Reports

### Report #1: All Tickets

Shows every ticket with rows color-coded by priority (warm=high, cool=low). Closed tickets are neutral gray regardless of priority.

| Priority | Color | Hex |
|----------|-------|-----|
| Immediate | Warm red | `#8b3a3a` |
| High | Warm orange | `#8b5a2a` |
| Medium | Muted olive | `#4a5a2a` |
| Low | Cool blue-teal | `#2a4a5a` |
| Zero | Cool blue-gray | `#3a3a5a` |
| Empty/null | Neutral gray | `#3a3a3a` |
| Closed (any priority) | Neutral gray | `#3a3a3a` |

**Columns:** `#`, `mtime`, `type`, `status`, `priority`, `severity`, `subsystem`, `title`

**SQL:**
```sql
SELECT
  CASE WHEN status='Closed' THEN '#3a3a3a'
       WHEN priority='Immediate' THEN '#8b3a3a'
       WHEN priority='High' THEN '#8b5a2a'
       WHEN priority='Medium' THEN '#4a5a2a'
       WHEN priority='Low' THEN '#2a4a5a'
       WHEN priority='Zero' THEN '#3a3a5a'
       ELSE '#3a3a3a' END AS 'bgcolor',
  substr(tkt_uuid,1,10) AS '#',
  datetime(tkt_mtime) AS 'mtime',
  type,
  status,
  priority,
  severity,
  subsystem,
  title
FROM ticket
```

### Report #2: Open Tickets

Shows only tickets with `status='Open'`, color-coded by priority (warm=high, cool=low). No status column (redundant since all are Open).

Same priority color scheme as All Tickets. Since all tickets shown are Open, all get priority coloring.

**Columns:** `#`, `mtime`, `type`, `priority`, `severity`, `subsystem`, `title`

**SQL:**
```sql
SELECT
  CASE WHEN priority='Immediate' THEN '#8b3a3a'
       WHEN priority='High' THEN '#8b5a2a'
       WHEN priority='Medium' THEN '#4a5a2a'
       WHEN priority='Low' THEN '#2a4a5a'
       WHEN priority='Zero' THEN '#3a3a5a'
       ELSE '#3a3a3a' END AS 'bgcolor',
  substr(tkt_uuid,1,10) AS '#',
  datetime(tkt_mtime) AS 'mtime',
  type,
  priority,
  severity,
  subsystem,
  title
FROM ticket WHERE status='Open'
```

## Color Scheme Rationale

The repo uses a customized `darkmode` skin with overrides for report table text colors. The built-in darkmode CSS forces `color: black` on report cells; the custom skin CSS overrides this to `color: #d4d4d4` (light gray) and `background-color: #2a2a2a` on ticket display cells. Form inputs on ticket pages are also styled dark.

Colors follow a warm→cool gradient by priority:
- **Immediate/High** → warm reds/oranges (urgent, attention-grabbing)
- **Medium** → neutral olive (middle ground)
- **Low/Zero** → cool blues/teals (calm, low urgency)
- **Closed/empty** → neutral gray (de-emphasized)

When changing colors, ensure:
- Colors are dark/muted enough for dark backgrounds (R/G/B values below ~140)
- Warm colors for high priority, cool for low
- Closed tickets always neutral gray
- Each priority level is visually distinct from its neighbors

## Ticket Schema

These are the valid values for each ticket field (defined in `ticket-common` config):

| Field | Values |
|-------|--------|
| type | `Code_Defect`, `Build_Problem`, `Documentation`, `Feature_Request`, `Incident` |
| priority | `Immediate`, `High`, `Medium`, `Low`, `Zero` |
| severity | `Critical`, `Severe`, `Important`, `Minor`, `Cosmetic` |
| resolution | `Open`, `Fixed`, `Rejected`, `Workaround`, `Unable_To_Reproduce`, `Works_As_Designed`, `External_Bug`, `Not_A_Bug`, `Duplicate`, `Overcome_By_Events`, `Drive_By_Patch`, `Misconfiguration` |
| status | `Open`, `Fixed`, `Closed` |
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
