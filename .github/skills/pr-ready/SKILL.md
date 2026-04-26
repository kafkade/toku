---
name: pr-ready
description: >
  Prepare a pull request: generate a PR description using the repo's template,
  update the Unreleased section of CHANGELOG.md with user-facing changes, and
  copy the PR description to the clipboard. Invoke when the user asks to
  "generate a PR description", "describe this PR", "write PR notes",
  "prepare a PR", "pr ready", or "prep this PR".
---

# PR Ready — Description + Changelog

Prepare a branch for pull request: generate a PR description from the diff AND update the changelog, then copy the PR description to the clipboard.

## Steps

### Phase 1: Gather context

1. **Detect the base branch**
   Run `git remote show origin` or check for `main` / `master` to determine the default branch.

2. **Identify the current branch**
   Run `git branch --show-current` to get the feature branch name.

3. **Gather the diff context**
   - Run `git log <base>..<current> --oneline` to get the commit list.
   - Run `git diff <base>..<current> --stat` to get the file change summary.
   - For each changed source file, read the diff to understand what changed.
   - Skip binary files, lock files, and generated files.
   - Note which crates are affected: `toku-core/`, `toku-db/`, `toku-import/`, `toku-meta/`, `toku-cli/`, `toku-export/`, `docs/`.

4. **Read the PR template**
   - Look for `.github/pull_request_template.md` in the repository.
   - Use its exact structure (sections, checkboxes) as the PR description format.
   - The template has a **Crate** section and a **Data Integrity Checklist** — fill both accurately.

5. **Read the current changelog**
   - Read `CHANGELOG.md` and note the existing `## [Unreleased]` section contents.
   - Understand the Keep a Changelog format used (Added, Changed, Deprecated, Removed, Fixed, Security).

### Phase 2: Run quality checks

1. **Always run markdown linting first** (every PR touches or could touch docs):

   ```sh
   npx markdownlint-cli2 "**/*.md"
   ```

   The repo-root `.markdownlint-cli2.jsonc` auto-configures rules and excludes
   build artifacts (`target/`, `node_modules/`, `pkg/`).

   If there are errors, **fix them before proceeding**. Do not generate the PR description until linting passes.

2. **Always run Rust checks if any Rust file changed** (or if any Cargo workspace member is affected):

   Run each step separately so failures are easy to identify:

   ```sh
   cargo fmt --check
   cargo clippy --workspace -- -D warnings
   cargo test --workspace
   ```

   If `cargo fmt --check` fails, run `cargo fmt` to fix, then re-run all three
   checks from the top. If `cargo clippy` fails, fix the warning, then re-run
   **all three checks from the top** (including `cargo fmt --check` — code fixes
   can introduce formatting changes). Only proceed once all three pass in a
   single run with no fixes in between.

   **Toolchain version gap:** CI uses the latest stable Rust, which may have
   newer clippy lints than the local toolchain. If CI fails with a clippy error
   that passed locally, fix it immediately — do not assume local-passing means
   CI-passing.

3. **If any check fails**: fix the issues, then **re-run ALL checks from step 1** — not just the one that failed. A clippy fix can break formatting; a formatting fix can break tests. Only proceed to Phase 3 once every check passes in a clean run with no intervening edits.

### Phase 3: Generate the PR description

1. **Write the PR description** using the PR template structure:
   - **Description section**: Write a clear summary of what the PR does. Include:
     - A one-line summary of the purpose
     - A "What's included" subsection listing key changes with brief explanations
     - Reference specific files/modules only when it adds clarity
   - **Related Issues**: Check commit messages for issue references (#123). If none, leave placeholder.
   - **Type of Change**: Check the appropriate box(es) based on diff content.
   - **Crate**: Check the appropriate box(es) based on which workspace crates were touched.
   - **Data Integrity Checklist**: Evaluate each item:
     - Does the change send user data to a server? (must not without opt-in)
     - Are import operations idempotent? (must be)
     - Are user metadata edits protected from auto-overwrite? (must be)
     - Do new fields track provenance? (must)
     - Is export round-trip preserved? (must be, if applicable)
     - If the PR is documentation-only or infrastructure-only, check all data integrity items as passing.
   - **Checklist**: Mark items based on the results from the quality checks step.

2. **Quality guidelines for the PR description**
   - Do NOT reference internal planning documents (roadmap phases, ADR numbers) — describe actual changes
   - Write from the user/contributor perspective
   - Be specific about what was added/changed
   - Keep the description concise but complete — aim for 15-30 lines

### Phase 4: Update the changelog

1. **Identify user-facing changes** from the diff. A change is user-facing if it:
   - Adds a feature the user can see or interact with
   - Fixes a bug the user could encounter
   - Changes behavior the user would notice (CLI output, search results, import behavior)
   - Adds or changes configuration options
   - **Is NOT user-facing**: refactoring, CI changes, test additions, internal restructuring, dependency updates, code style fixes, documentation-only changes

2. **Categorize each change** using Keep a Changelog categories:
   - **Added** — new features or capabilities
   - **Changed** — changes to existing functionality
   - **Deprecated** — features that will be removed
   - **Removed** — features that were removed
   - **Fixed** — bug fixes
   - **Security** — vulnerability fixes

3. **Update the `## [Unreleased]` section** of `CHANGELOG.md`:
   - **Append** new entries to the existing Unreleased section — do NOT delete what's already there
   - If a category header (e.g., `### Added`) already exists with entries, add new entries below the existing ones
   - If a category header doesn't exist yet, add it
   - Write entries as concise, user-facing descriptions — no implementation details
   - Each entry starts with a `-` list marker (markdown list item)
   - Do NOT include entries for: CI changes, refactoring, dependency bumps, test-only changes, documentation-only changes

4. **Changelog entry style guide**
   - ✅ Good: `- Goodreads CSV import with dry-run, dedup, and field mapping report`
   - ✅ Good: `- Fixed ISBN-13 validation rejecting valid checksums`
   - ✅ Good: `- Reading progress tracking with page, percentage, and chapter modes`
   - ❌ Bad: `- Refactored toku-db query layer` (not user-facing)
   - ❌ Bad: `- Added unit tests for import dedup` (not user-facing)
   - ❌ Bad: `- Implements Phase 1 from roadmap` (references internals)

### Phase 5: Output

1. **Copy the PR description to the clipboard**
   - Use PowerShell `Set-Clipboard` (Windows), `pbcopy` (macOS), or `xclip` (Linux)
   - Confirm to the user that the description has been copied

2. **Suggest a PR title**
   - Based on the changes, suggest a conventional-commit-style PR title
   - Format: `feat: add Goodreads CSV import` or `fix: correct ISBN-13 checksum validation`
   - For multi-crate changes, use the primary crate: `feat(import): add Calibre metadata.db parser`

3. **Show a summary** to the user:
   - The suggested PR title
   - Confirmation that the PR description is on the clipboard
   - A summary of what was added to CHANGELOG.md (list the new entries)
   - Note any changelog entries that were already present and preserved
