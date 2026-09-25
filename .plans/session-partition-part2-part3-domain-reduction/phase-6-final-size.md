# Phase 6 Final `jinn-domain` Size

Measured after the complete session partition, consumer migration, and final
verification gates.

## Primary metric: `tokei` Rust code lines

| Measurement | Before | After | Removed | Reduction |
| --- | ---: | ---: | ---: | ---: |
| Rust code | 53,763 | 34,093 | 19,670 | 36.6% |
| Rust comments | 7,569 | 4,958 | 2,611 | 34.5% |
| Rust blank | 9,069 | 5,966 | 3,103 | 34.2% |
| Rust physical | 70,401 | 45,017 | 25,384 | 36.1% |
| Rust files | 240 | 191 | 49 | 20.4% |

Command:

```text
tokei crates/jinn-domain
```

The final tree reports 191 Rust files, 45,017 physical lines, 34,093 code
lines, 4,958 comments, and 5,966 blanks.

## Secondary metric: all Rust physical source

| Measurement | Before | After | Removed | Reduction |
| --- | ---: | ---: | ---: | ---: |
| All Rust physical lines | 77,556 | 49,330 | 28,226 | 36.4% |
| All Rust files | 240 | 184 | 56 | 23.3% |

The baseline values come from the Phase 1 inventory measured on 2026-09-25.
The final values use the same `find`/`wc -l` physical-line method under
`crates/jinn-domain/src`.

The baseline's additional strict source-physical classifier is not reported
because its exact helper code was not retained. The primary `tokei` metric and
the directly reproducible all-source physical metric demonstrate measurable
removal without mixing unlike counting methods.
