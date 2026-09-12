# Repository Instructions

## Rust Rules

- In Rust code, do not introduce `unwrap()` or `expect()`.
- Allowed exceptions:
- Tests may use `unwrap()` or `expect()` when it keeps the test focused and readable.
- Lock acquisition may use `unwrap()` only when the locking API makes that the practical option and the failure mode is poison handling rather than normal control flow.
- Outside those exceptions, propagate errors, handle them explicitly, or use safer fallbacks instead of `unwrap()` and `expect()`.

## Editing Hygiene

- Do not introduce formatting-only changes.
- Do not run repository-wide formatters or reflow unrelated code unless the
  user explicitly asks for formatting.
- Keep diffs limited to semantic changes required for the task.

## File Size Rule (mandatory)

- No Rust source file may exceed **300 lines** (blank lines and comments
  included). Split into a directory module (`foo.rs` -> `foo/mod.rs` +
  `foo/<topic>.rs`) grouped by responsibility, re-exporting from the module
  root so call sites do not change in the same commit.
- Splits are mechanical and live in their own commit; `cargo build --bins` and
  `cargo test` must pass with no new warnings.
- Touching a legacy file still over 300 lines: split it first in its own
  commit, then make the change.

#### Commit granularity

Every change lands as a series of small, incremental local commits; one huge
commit (dozens of files, thousands of changed lines) cannot be reviewed.

* Splitting an oversized file is not "one file, one commit". Each commit moves
  out only 1-3 new files and changes roughly 300-600 lines. Commit the
  directory plus the `mod.rs` / barrel shell first, then move code block by
  block, and delete what is left of the original file in the final commit.
* Every commit must build (`cargo check` with the `flutter` feature, or
  `flutter analyze`) with no new diagnostics, so `git bisect` stays usable.
* The first line of the commit message says what moved (for example
  `refactor(client): move audio handler out of client.rs`); the body lists the
  moved items and any visibility or import changes that were required.
* Feature work follows the same rule: one logical unit per commit (a config
  key, a platform function, a UI component and a translation key are separate
  commits).
* Large commits that already exist are left as they are (no rebase, no amend);
  the rule applies from the next commit on.
