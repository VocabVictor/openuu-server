# Repository Instructions

## Verification is command-line only

No end-to-end or GUI automation on virtual machines or any desktop: no
synthesized clicks, no screenshot harvesting, no driving an RDP session.
Verify with `cargo test`, `flutter test` / widget tests, the build-machine
`check.ps1`, command-line interfaces (`--get-id`, `--option`,
`--import-config`, `--connect` and its log outcome, service logs, `curl`
against HTTP endpoints) and assertions on configuration files and logs. An
installer is verified with a silent install (`msiexec /qn`) followed by
service status, configuration file content and service log checks. When a
screen has to be judged by eye, produce one screenshot for the user and stop;
do not automate the interaction.

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
* Several sessions share this working tree and its index. Stage only your own
  new files (`git add <new file>`; a pathspec commit does not pick up untracked
  files), then commit with an explicit pathspec, `git commit -m "..." --
  <your paths>`, which takes those paths from the working tree and ignores
  whatever else is staged. A bare `git commit`, `git commit -a` or
  `git add -A` sweeps other people's staged work into your commit. Check
  `git show --stat HEAD` afterwards.

## Commit messages and remote branches

* A commit message carries no AI attribution: no `Co-Authored-By: Claude ...`,
  `Claude-Session: ...`, `Generated with Claude Code` or similar trailer.
  The author is the human account; tooling is not credited in history.
* GitHub holds only `master`. Nobody pushes any other branch there; work in
  progress lives in local branches and on the build machine's bare repository.
  Linux verification runs through `C:uild\check-linux.ps1` (WSL Debian on
  the build machine); the `linux-check.yml` workflow is only a backstop on
  pushes to `master`.

## Documentation and test data

* Documents, commit messages and test fixtures must not contain real IP
  addresses, host names, account names, cloud instance ids or device ids.
  Use placeholders: documentation address ranges (192.0.2.0/24, 198.51.100.0/24,
  203.0.113.0/24, or a private range when the test needs one), made-up names
  (`alice`, `<user>`) and ids such as `123456789`. The repository is published.
