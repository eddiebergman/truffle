# Storage and feature development

- Keep storage access behind backend module APIs. UI code and agent tools must not read or write state files or database tables directly.
- Store stable identities and relationships in the core model. Preserve IDs across imports, exports and migrations.
- Keep experimental feature payloads in a feature-owned namespace with an explicit payload version. Use bounded JSON documents until a real query or constraint requires structured columns. Do not add a SQL table or column for each prototype field.
- Once SQLite is introduced, use it through the storage API, including for versioned JSON feature documents. Do not create parallel file stores to avoid SQL migrations.
- Promote fields into indexed columns only for demonstrated query or integrity needs. Keep provider-specific and evolving fields in JSON. Validate writes in the owning feature module.
- Do not use one unbounded JSON document for all feature state. Set size and retention limits. Keep artifacts, terminal output and large logs outside the metadata database.
- New schema migrations must be numbered, transactional and tested with older fixture data. Never edit an applied migration. Refuse databases from unsupported newer schemas.
- Before the JSON-to-SQLite cutover, create a temporary database, import all supported records, preserve unknown namespaced payloads, and validate counts, IDs and relationships. Activate only after validation succeeds. Preserve the original JSON and a migration receipt. A failed import must leave the old store usable. Re-running a failed migration must not duplicate records.
- Back up before schema changes. Use a consistent SQLite backup mechanism while the database is open; do not copy only the main file while WAL writes are active. Recovery must not silently discard writes made after a backup.
- Use one local service as the metadata writer. Keep write transactions short. Add indexes for actual queries and test pagination with realistic history sizes.
- Keep settings separate from operational chat and review history. Use versioned settings, atomic writes and revision checks. Agents can apply routine settings. Commands, hooks, credentials, access and trust changes require a user approval path.
- Packaged defaults must not contain personal review skills, company accounts or company quick actions. Migrate existing preferences explicitly; do not infer them from a developer's home directory.
