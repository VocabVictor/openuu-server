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
