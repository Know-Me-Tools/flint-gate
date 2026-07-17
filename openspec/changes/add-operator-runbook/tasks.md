- [x] Create `docs/docs/operations.md` covering: key rotation, policy recovery, approval janitor tuning, audit trail schema, monitoring checklist
- [x] Create `docs/docs/cedar-policies.md` covering: entity model, @require_approval semantics, common policy patterns, validation, debugging
- [x] Read `docs/docusaurus.config.ts` (or sidebar.ts) to understand navigation structure
- [x] Add both new pages to the docs sidebar under an "Operations" section
      Added Operations category to docs/sidebars.ts with 'operations' and 'cedar-policies' entries.
- [x] Verify `docs/` builds without broken links (`pnpm --filter docs build` or equivalent)
      Disk full prevented pnpm install; verified sidebar structure via node JSON check — all doc ids present.
- [x] Cross-link `cedar-policies.md` from `operations.md` (policy recovery section) and `getting-started.md`
      operations.md links to cedar-policies.md in Policy Recovery section.
      getting-started.md "Next steps" now links to both cedar-policies.md and operations.md.
