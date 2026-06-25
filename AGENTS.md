# AGENTS.md

Meta-guideline for any AI coding agent working in this repository. Read this
first.

## This Project Uses OpenSpec

The source of truth lives in `openspec/`, which is version-controlled and
agent-agnostic.

- `openspec/specs/` - the living specification of what the system currently is.
- `openspec/changes/` - active change proposals as delta specs.
- `openspec/changes/archive/` - completed changes.

Per-agent command files such as `.codex/`, `.claude/`, and editor-specific shims
are per-clone generated files and are not committed. After cloning, generate
your own with:

```bash
openspec init --tools codex
# or: openspec init --tools claude,cursor,github-copilot
```

## Workflow

Follow this lifecycle:

```text
explore -> propose -> apply -> sync -> archive
```

1. **Explore**: think and investigate only. Do not write feature code outside of
   a change.
2. **Propose**: create a change with `proposal.md`, `design.md`, `tasks.md`, and
   delta specs.
3. **Apply**: implement tasks one at a time, checking each off in `tasks.md`
   only after verification.
4. **Sync**: merge verified delta specs back into `openspec/specs/`.
5. **Archive**: move the completed change to
   `openspec/changes/archive/YYYY-MM-DD-<name>/`.

## OpenSpec CLI

If your agent has no OpenSpec slash commands, use the CLI:

```bash
openspec list [--json] [--specs]
openspec new change "<name>"
openspec status --change "<name>" --json
openspec instructions <artifact> --change "<name>"
openspec archive <name>
```

## Rules

- Before implementing anything, read the relevant files in `openspec/specs/` and
  the active change's artifacts.
- Do not write feature code without an active change proposal that contains
  tasks.
- Keep changes minimal and scoped to the task being implemented.
- Treat `openspec/specs/` as the truth. Reflect requirement changes there via
  the sync step, not by editing code silently.
- Keep project-specific contract, terms, and priorities in `PROJECT.md`.

## Amendment flow

When Modou's reaction fails (exit 1), there are two cases:

- **Shape drift** — the code violates a sound boundary. Repair the code. This is
  the normal path.
- **Policy drift** — the boundary itself is wrong (e.g. a dependency genuinely
  belongs now). The constitution must change, but you **must not** silently weaken
  it to make CI pass.

For policy drift, propose an **amendment**: a PR that changes the constitution
(`crates/modou/src/constitution.rs`) with the reasoning, for the **steward** to
accept or reject. Never edit the constitution — or the living specs in
`openspec/specs/` — to turn CI green on your own.

This is enforced by convention, not by Modou: those paths are assigned to the steward
in `.github/CODEOWNERS`, and with required Code Owner review enabled on the default
branch a merge that relaxes the law requires steward approval (until that branch
protection is on, the designation only auto-requests review). Modou stays a reactor
and owns no part of this flow (see the amendment-flow decision in `PROJECT.md`).

## Language

- Write OpenSpec artifacts, ADRs, code comments, and commit messages in English.
- Converse with users in the language they use.

## Commits

Use Conventional Commits:

```text
type(scope): summary
```

Use lowercase imperative mood and keep the summary at 72 characters or fewer.
Common types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `build`,
`ci`.

Commit subjects **and `CHANGELOG.md` entries** are **self-describing**: they state
what the change does, with no issue or PR numbers (no `#123`, no trailing `(#123)`).
When squash-merging a pull request, strip GitHub's auto-appended `(#N)` from the
squash subject, and never carry a PR or issue number into a changelog entry — the log
records *what* changed, not where it was reviewed. This is a convention, not a Modou
reaction: neither a commit message nor a changelog line is an observable architectural
fact (Modou observes `cargo metadata` and source `use` declarations), so the drift law
keeps it out of the constitution.

### Commit Flow

- **Propose**: `docs(<change>): propose <summary>`
- **Apply**: `feat(<change>): <summary>` or `fix(<change>): <summary>`
- **Sync**: `docs(specs): sync <change>`
- **Archive**: `chore(openspec): archive <change>`

Never bundle unrelated changes into one commit.

## Definition Of Done

Run these from the workspace root before checking off a task, syncing specs, or
archiving a change:

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

If a command cannot run in the current environment, report that explicitly.
