# Device IR configuration API

A normal device may include `"ir":{"codeset":"device-specific-id"}` alongside
its network `integration`. A new IR-only device uses `{"via":"none"}` and the
same `ir` field. Older direct IR integrations and IR connection resources remain
readable; editing their IR commands migrates that device to the new representation.

All routes require normal web UI authentication. Writes require `If-Match` with
the current configuration revision; missing revisions return 428, stale revisions
409. Successful writes return the complete configuration and updated ETag.

- `GET /api/rooms/{room}/devices/{device}/ir` returns `codeset`, canonical `text`,
  parsed `commands`, and `supplemental`/`legacy` flags. No attachment returns a
  null codeset and empty commands/text. A missing installed file includes a
  repairable `error` while retaining the assigned ID.
- `PUT` to that route with `{"text":"volume-up nec 4 2\n"}` validates commands,
  writes a fresh private codeset, then attaches it in one configuration commit.
- `DELETE` removes IR commands while preserving the network integration. Legacy
  IR-only integrations become `none`.
- `POST /api/rooms/{room}/devices/ir` with `name`, optional `kind`, and `text`
  atomically creates an IR-only device plus attachment. The created-ID header
  matches the ordinary device creation endpoint.

Each save publishes a new uniquely named codeset. Other devices and existing
references keep their original file; configuration write failures remove the
unpublished new file. Historical files are intentionally retained after successful
edits for rollback/reference safety. No route above transmits IR.

Codeset function names identify explicit overrides. Runtime must look up the
exact assignment and otherwise use the existing network integration; importing a
power toggle never implies discrete power-on or power-off. The model's
`Function::supports_device` is configuration validation, not proof that a
particular command exists in the installed codeset.
