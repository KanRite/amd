# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Scope

`agile-md` is a tiny **filesystem Kanban**. It ships two binaries over one
library — **`amd`**, the command line, and **`amdui`**, the desktop board —
which manage markdown task files moved between `backlog/`, `todo/`, `doing/`
and `done/` directories. Board data lives in each *consuming* repo at `<repo-root>/tasks/`;
they're installed globally and operate on whatever repo you're in. Rust +
`git`, shipped as static binaries — no runtime, no daemon, no database.
Design principles: **status is the folder**, **git history is the audit trail**
(moves are `git mv`), tasks are self-describing markdown **rendered from
MiniJinja templates**.

## Layout

A **cargo workspace of three crates** (ADR-0004), so the command line and the
desktop board share one implementation of the board and the boundary is
enforced by the dependency graph rather than by review:

```
crates/amd-lib/   the library (`agile_md`) — everything that touches the board
crates/amd-cli/   → binary `amd`    (clap, inquire, the $EDITOR scratch file)
crates/amd-ui/    → binary `amdui`  (egui/eframe)
```

The library keeps the crate name `agile_md`, so every `use` in the workspace is
unchanged. Still to move (docs/workspace-split.md): `gui/` belongs in amd-ui.

- `crates/amd-lib/src/lib.rs` — the library: `board`, `board_settings`,
  `branch`, `create`, `git`, `group`, `project_trust`, `registry`,
  `settings`, `task`, `templates` (and `gui` behind the feature, pending
  phase 4). **No `render`** — drawing is a front end's business.
- `crates/amd-lib/src/create.rs` — making a ticket, split into `Draft::prepare` (resolve and
  render, touching nothing) and `Draft::write` (file, child link, backlinks).
  That gap is what lets the CLI put the rendered ticket in `$EDITOR` before
  anything is written, while the desktop board writes what it already has.
- `crates/amd-lib/src/gui/` — the desktop board (feature `gui`, on by default): egui/eframe,
  the same stack as rustmark. `richtext.rs` edits a body line by line;
  `settings.rs` holds the text size and colour scheme.
- `crates/amd-lib/src/group.rs` — epics and sprints: the folders a backlog is divided into.
- `crates/amd-lib/src/settings.rs` — the shared config file, which keeps keys it does not
  recognise so one writer cannot delete another's.
- `crates/amd-lib/src/board_settings.rs` — the board-local settings file
  (`<repo-root>/.settings`, git-tracked, hidden to match the legacy
  `.next-id` file it partly replaces): the declared, immutable `project`
  identity and the `next-id` ticket counter.
- `crates/amd-lib/src/project_trust.rs` — a per-machine, trust-on-first-use
  check that a board's declared `project` hasn't changed unexpectedly since
  `amd` last saw it.
- `crates/amd-lib/src/import.rs` — `amd import <path>`: reads a tasks
  directory tree, treating it as a read-only `Board` (`Board::at`), and
  copies each ticket into the current board — status kept, id renumbered
  against the destination's own counter, `parent`/`related` remapped when
  both ends are part of the same batch, same-titled tickets (within the
  batch or against the destination) tagged `DUPLICATE`. `archive/` isn't a
  destination. A ticket that fails to parse (no frontmatter, unreadable) is
  skipped and reported, not a hard failure for the whole batch. `--dry-run`
  computes and reports everything without writing or spending an id.
- `crates/amd-cli/src/main.rs` — the `amd` binary: clap CLI and command handlers; it also owns
  `form.rs`, `value.rs`, `completions.rs` and `init.rs`, which are CLI-only.
- `crates/amd-cli/src/init.rs` — `amd init <project>`: finds or creates a
  private `<project>-tickets` GitHub repository (via `gh`, as a child of the
  current directory), clones it, scaffolds the board, and registers it.
  **Refuses to run from inside an existing git repository** — a guard, not
  an inference: an earlier version tried placing the new repo as a sibling
  of whatever repo you happened to be standing in, which was a real source
  of confusion. `--local-only` is the command for "inside a repo" — it
  skips all of the above and just scaffolds here, the way bare `amd init`
  always has.
- `crates/amd-ui/src/main.rs` — the `amdui` binary: ~50 lines that call
  `agile_md::gui::run()`, so the board is something you launch rather than a
  subcommand you remember. `amd gui` stays — it's the same call, and it's what
  a prebuilt install has (see below), which is why `amd-cli` keeps a `gui`
  feature (on by default) rather than shedding egui entirely.
- `crates/amd-lib/src/board.rs` — board discovery (`<repo-root>/${AMD_DIR:-tasks}`), the
  `Column` enum, task lookup, moves, `create` (scaffolding — the interactive
  "create one?" prompt itself is `ensure_board()`, CLI-owned).
- `crates/amd-lib/src/task.rs` — `Task` (path, column, id, stem), frontmatter `meta` lookup,
  `slugify` (transliterates via `deunicode`, so a title in any script still
  makes a plain-ASCII filename and branch; symbols stay separators so an emoji
  doesn't become a word).
- `crates/amd-lib/src/templates.rs` — the MiniJinja `Environment`, built-in templates, board
  overrides, `TaskContext`, `required_extras`.
- `crates/amd-cli/src/form.rs` — every interactive prompt, built on `inquire` (text, select,
  confirm, task picker, label autocomplete).
- `crates/amd-lib/src/branch.rs` — the branch-type taxonomy (`feature`, `bugfix`, `hotfix`,
  `release`, `chore`, overridable with `AMD_BRANCH_TYPES`) and git ref-name
  validation.
- `crates/amd-cli/src/render.rs` — board output: a rich tree (`richrs`) on a
  terminal, plain greppable text everywhere else. It lives in the CLI because
  it is the CLI's output; `richrs` is not an amd-lib dependency.
- `crates/amd-cli/src/completions.rs` — `amd completions [SHELL]`, generated from the clap
  command with `clap_complete`; `$SHELL` detection and install hints.
- `crates/amd-cli/src/value.rs` — `TypedValueParser`s that advertise their possible values, so
  completions and `--help` list them while validation stays ours.
- `crates/amd-lib/src/git.rs` — thin wrappers over the `git` CLI: repo
  discovery and file state (`repo_root`, `is_tracked`, `mv`, `config`),
  branches (`current_branch`, `branch_exists`, `switch_branch`,
  `commits_ahead`, `branch_pushed`, `delete_branch`, `set_head_branch`), and
  a small set for `amd init`'s tickets-repo bootstrap (`remote_url`,
  `commit_all`, `push`).
- `crates/amd-lib/templates/*.md.jinja` — the built-in templates, `include_str!`'d into the
  binary.
- `install.sh` — downloads the prebuilt `amd-<target>.zip` from the GitHub
  release, or builds from source in a clone (`--from-source`, `--dir`,
  `--version`). Keep the two paths in sync. Both paths install `amdui` as well,
  guarded by `if [ -f ... ]` — `--version` can pin a release made before the
  zip carried both.
- `tests/test.sh` — assert-based end-to-end suite; builds with cargo and spins
  up temp git repos. Unit tests live next to the code in `#[cfg(test)] mod tests`.
- `.github/workflows/ci.yml` — fmt + clippy + cargo test + tests/test.sh, and
  shellcheck for the shell files.
- `.github/workflows/release.yml` — on a `vX.Y.Z` tag, calls
  `lite-actions/rust-release` to build and attach `amd-<target>.zip` per platform.
  `bin-name: "amd amdui"` — this needs rust-release to accept a list of
  binaries. **That change was lost in a laptop crash and must be redone**
  (lite-actions/rust-release); until it is, a tagged release fails looking for
  a binary literally named "amd amdui".
- `docs/adr/` — architecture decision records: why the tool is shaped as it
  is, and what each decision costs. `docs/workspace-split.md` is the plan for
  the amd-lib / amd-cli / amd-ui split (ADR-0004).
- `README.md`, `LICENSE` (MIT).

## How `amd` works (architecture)

- `Board::locate()` resolves the board as `$(git rev-parse --show-toplevel)/${AMD_DIR:-tasks}`,
  so `amd` works from any subdirectory; it errors if not inside a git repo.
- Tasks are `NNN-slug.md`. `Board::next_id()` takes whichever is higher of the
  counter (the board-local settings file's `next-id` key, `<repo-root>/.settings`
  — a board that predates the move still has the legacy `<board>/.next-id`
  file instead; the first read migrates its value into settings and deletes
  it, so this only ever happens once per board) and one past the highest id
  in the columns. The counter is the
  authority — it remembers ids whose tickets have gone, archived or deleted,
  which no scan can — and the scan is the safety net, so a lost or badly
  merged counter can't hand out an id already in use. `record_id()` runs in
  `Draft::write`, not `prepare`: an abandoned draft leaves no gap. The
  archive isn't scanned; the counter covers it.
- The columns are `backlog`, `todo`, `doing`, `done` (`Column::ALL`, in that
  order). **`amd new` lands in `backlog/`** — new work is raised, then pulled
  into `todo` when someone means to do it. `tests/test.sh` drifted from the
  tool for exactly this reason, so it is worth stating plainly.
- `Board::find(<ref>)` resolves a numeric id or a unique slug substring to one
  task (errors on 0 or >1 matches).
- `Board::move_task()` uses `git mv` for tracked files (history follows the
  rename), else `fs::rename`. It does **not** commit — the user commits the move.
  It returns where the ticket landed and **prints nothing** — `amd` prints
  `moved X -> done/` from `move_and_report`, and the board says it with a card
  that has moved.
- **Destination-as-verb dispatch** (ADR-0006): `amd todo`/`doing`/`done`/
  `backlog`/`abandon <ref>` move a ticket there; leave off `<ref>` and the
  same verb lists that destination instead of moving anything — one command
  name serves both. The same works for any epic by name (`amd checkout 7`
  files it, `amd checkout` lists it). Entering `todo` is the one with a side
  effect: it creates or switches to the ticket's branch — `amd start` doesn't
  exist any more, this replaced it. `amd abandon` also deletes the ticket's
  branch if it's still empty, since a ticket can be abandoned from `todo/`
  before real work starts.
- **Tag filtering**: `amd tag <tag>` lists every ticket carrying `<tag>`,
  across every column, via `Board::tasks_tagged()`/`Task::has_tag()`. Every
  listing form also takes `--tag <tag>` to narrow it down: `amd board`,
  `amd ls [column]`, a bare destination verb (`amd todo`, `amd backlog`, …)
  and an epic by name (`amd checkout --tag urgent`) all thread it through to
  `render::columns()`/`render::named()`, which filter before building the
  tree. Moving a ticket (`<ref>` given) ignores `--tag` — it only applies to
  the no-ref, listing form. The `Verb` external-subcommand path (epic names)
  parses `--tag`/`-t` out of its raw args by hand, since clap hands that
  whole tail over as an opaque `Vec<String>`.
- **Leases** (`docs/adr/0007-concurrency-and-locking.md`, currently only
  surviving at `~/develop/github/agile-md/to-sort/old-amd-docs/adr/` —
  `docs/adr/` was never restored into this repo after the git-history reset):
  `locked-by`/`locked-until` frontmatter, stored as an **absolute expiry**
  rather than a duration so every check (`Task::write_checked`,
  `Board::move_task`/`archive`/`set_epic`) is a self-contained
  `now > locked_until` comparison against data already in the file — no
  `Board` handle needed except at the one place a duration is actually
  chosen (`Task::take_lease`, called from `amd lock` and from
  `move_and_report`/`amdui`'s `apply()` on entering `todo`/`doing`, both of
  which already hold one). Identity is `git::config(repo, "user.email")` —
  `git.rs`'s `config()` takes an explicit repo path like every other
  function there, not the process's current directory, so a desktop board
  open across several registered repositories resolves each ticket's own
  identity correctly. `take_lease`/`release_lease` need no special-casing:
  both are ordinary `write_checked`-protected writes, and its generic "leased
  by someone else" gate already gives exactly the right semantics for each
  (blocked if someone else holds it; blocked unless *you're* the holder).
  Only `break_lease` (`amd unlock --force`) bypasses that gate — the one
  deliberate act ADR-0007 wants kept separate from an ordinary write.
  `board_settings::lease_hours()` resolves the duration (`AMD_LEASE_HOURS`,
  then the board's own `.settings`, then 8 hours) the same
  `settings::resolved()` precedence every other setting uses.
- `Board::archive()` (`amd abandon`) moves a ticket to `<board>/archive/`,
  which is **not** a column: it's off the board and its contents are
  gitignored. That means `git mv` would refuse the ignored destination and
  forcing it would put the ticket back into the history the `.gitignore`
  exists to keep out, so a tracked ticket is `git rm --cached`'d and then
  moved. Archived ids are never reused — that's the counter's job — because
  reusing one would silently repoint every `parent` and `related` that named
  it.
- `ensure_board()` (`crates/amd-cli/src/main.rs` — CLI-owned, not a `Board`
  method) runs before any board-requiring command: if there's no board, it
  prompts to create one when interactive (`stdin().is_terminal()`),
  auto-creates when `AMD_YES=1`, and otherwise errors (never hangs
  non-interactively).
- Env vars: `AMD_DIR` (board dir name, default `tasks`), `AMD_BRANCH_TYPES`
  (the branch types on offer), `AMD_YES` (force-create), `AMD_NO_INPUT` (never
  prompt), `AMD_NO_BRANCH` (never touch branches), `AMD_NO_REGISTER` (keep the
  repository list manual), `NO_COLOR`/`--plain` (plain board), `EDITOR` (for
  `amd edit`), `AMD_LEASE_HOURS` (lease duration, default 8), `AMD_WIP_TODO`/
  `AMD_WIP_DOING` (WIP limits, unset = unlimited). **`AMD_TYPES` is
  dead** — it was in `--help` long after nothing read it. `AMD_BRANCH_TYPES`,
  `EDITOR` and `AMD_NO_REGISTER` also fall back to the settings file
  (`branch-types`/`editor`/`no-register`) when unset — the env var wins when
  both are set (`settings::resolved()`). `AMD_LEASE_HOURS`/`AMD_WIP_TODO`/
  `AMD_WIP_DOING` follow the same precedence against the *board-local*
  `.settings`'s `lease-hours`/`wip-todo`/`wip-doing` keys instead — both are
  team decisions, not personal ones.

## Ticket fields (`crates/amd-lib/src/branch.rs`, `templates/ticket.md.jinja`)

- **One ticket, one template.** There are no ticket types any more: `admin` and
  `development` collapsed into fields you either fill in or don't.
- **`branch-type` is what gives a ticket a branch**, and it's empty by default.
  `branch-name` is derived from it and the title at creation and stored, so it
  can be edited before work starts. No branch type means no branch, and
  `amd todo` says so and leaves the working tree alone.
- The values are the **branch** convention (`feature`, `bugfix`, `hotfix`,
  `release`, `chore`), matching mrdoodles/conventional-validator's branch check
  directly — there's no longer a commit-type-to-branch-prefix mapping to keep in
  step. `AMD_BRANCH_TYPES` overrides the list.
- `assignee` says who's doing it, empty by default: tickets are created now and
  assigned later. There is deliberately **no second person field** — who created
  a ticket is already in `git log --follow`, so a `reporter` would duplicate
  history the tool is built on. It was briefly called `owner`; `Task::assignee`
  still reads that key.
- **Frontmatter keys are hyphenated, template variables are not** —
  `branch-type: {{ branch_type | yaml }}`. Jinja would read `branch-type` as a
  subtraction, so the variable has to be `branch_type`; the key is just text.
- `Task` reads the new keys and **falls back to the old ones** (`type`,
  `branch`, `owner`), so boards written before this change still list, enter
  `todo/` and filter correctly. There's no migration command; a ticket picks
  up the new shape when it's rewritten.
- Titles are validated against git's ref rules at creation — the title becomes
  the branch, so a title with nothing sluggable in it is rejected up front.
  `slugify` returns an empty string in that case; callers must reject it.
- **Entering `todo/` is what creates the branch now** (`cmd_enter_todo`,
  `crates/amd-cli/src/main.rs`) — this replaced the old `amd start`, retired
  outright, not kept alongside the new verbs. It takes the branch from
  `--branch`, else the ticket's `branch-name`, else derives one from
  `branch-type` and the title (`--no-branch` skips all of this). **Order
  matters**: `git mv` is staged first so the rename travels with the switch.

## Board rendering (`crates/amd-cli/src/render.rs`)

- Each column is a **tree**: `forest()` nests tasks by `parent`, ordered by id
  at every level. A child whose parent is in another column stays at the top
  level with a `^NNN` marker — the columns are the board's primary structure.
  `forest()` is a pure function over `(Task, Option<String>)` pairs, so it's
  unit-tested without touching the filesystem; parents are read once per column
  rather than per lookup (each read is a file read).
- Rich (`richrs::Tree`) only when stdout is a terminal and neither `--plain` nor
  `NO_COLOR` is set; otherwise the same tree as indentation, which is what
  `tests/test.sh` asserts on and what pipes into `grep`.
- Ids are **OSC-8 hyperlinks** in the rich path (`TERM=dumb` opts out), so
  clicking one opens the ticket. They are deliberately not in the plain path:
  escape sequences have no business in a pipe. Note this is why the board moved
  off `richrs::Table` — a table measures cell widths and would count the escape
  bytes, misaligning every border. (`richrs` draws the one-shot `amd board`
  printout only; the interactive board is egui.)
- A rendering failure falls back to plain output rather than erroring: never
  let cosmetics stop someone seeing the board.

## Forms (`crates/amd-cli/src/form.rs`)

- **Prompting is always optional.** `form::available()` gates every prompt on
  `stdin` *and* `stdout` being a terminal, plus `--no-input`/`AMD_NO_INPUT=1`.
  Non-interactively a missing value is an error with the usage line — the same
  contract the bash version had, and why the tool never hangs in CI.
- Optional arguments drive it: `amd new` with no title runs the full form
  (title, branch type, assignee, parent, related, tags), `-i` re-asks for what was
  passed, and `show`/`edit` with no `<ref>` open a task picker across every
  column (`resolve()`). Destination verbs (`todo`/`doing`/`done`/`backlog`/
  `abandon`) are different: no `<ref>` lists that destination instead of
  picking — one command name serves both (ADR-0006).
- **The form is derived from the template**: `templates::required_extras()`
  scans the source for `extra.<key>` / `extra["key"]`, and `cmd_new` prompts
  for each one that `--set` didn't supply. Adding a field to a template adds a
  question — there is deliberately no separate field registry to keep in sync.
- `tests/test.sh` exports `AMD_NO_INPUT=1` so the suite stays hermetic when run
  from a terminal; it covers the non-interactive halves of these paths. Drive
  the interactive halves by hand through a pty (`script -q /dev/null amd new`).
- Esc/Ctrl-C map to a plain `cancelled` error, not a panic or a partial task.
- **The form ends in the body.** `form::body()` renders the ticket into a temp
  file, opens `$EDITOR` on it (splitting the command so `code --wait` works)
  and returns what came back; `cmd_new` only writes to the board afterwards.
  A non-zero editor exit or an emptied file creates nothing. Deliberately not
  inquire's `Editor` prompt: that waits for an `e` keypress first, which is the
  stop this was meant to remove. Default on when the form ran or `-e`; off with
  `--no-edit` and whenever prompting isn't available, so scripts and CI never
  spawn an editor. Unit-testable without a TTY — pass `true`/`false` as the
  editor command.

## Templates (the reason this is Rust)

- Built-ins are compiled in via `include_str!` (`ticket`, `board-readme`).
  Anything in `<board>/templates/<name>.md.jinja` overrides or adds to them;
  `amd templates eject <name>` writes an editable copy there.
- The environment is deliberately configured: `UndefinedBehavior::Strict` (a
  typo'd variable is an error, not a blank line), auto-escape **off** (output is
  markdown, not HTML), `keep_trailing_newline`, `trim_blocks`, `lstrip_blocks`.
- Custom `yaml` filter quotes/escapes frontmatter values, so a title containing
  `"` can't corrupt the block. Use it for every frontmatter value.
- `TaskContext` is the contract with templates (`id`, `number`, `title`, `slug`,
  `branch_type`, `branch_name`, `owner`, `parent`, `parent_link`, `related`, `tags`, `created`, `timestamp`, `column`,
  `author`, `email`, `board`, `template`, `extra`). Adding a field is additive;
  renaming one breaks every user template — treat it as a public API.
- Template errors are flattened by `render_error()` (message + line + cause
  chain + MiniJinja debug info); without that only the top line survives.

## Several repositories (`crates/amd-lib/src/registry.rs`)

- The registry is **a list of repository paths and nothing else**, one per line
  at `$XDG_CONFIG_HOME/agile-md/repos`. There is deliberately no store of
  tickets: the markdown is the source of truth, it changes under you on every
  checkout and pull, and an index would be stale the moment it was written.
  Reading a board is a directory listing and a few small files.
- **The list fills itself**: `registry::remember()` runs whenever a command
  resolves a board, writing only the first time a repository is seen, so the
  boards you work on accumulate across sessions. Best effort — a read-only
  config directory must not stop `amd board`. `AMD_NO_REGISTER=1` opts out, and
  only then does `amd repos remove` stick.
- **`amd repos` and `amd completions` run before the board is resolved.** Asking
  what's on the list must not add to it, and both have to work outside a repo.
  That bug shipped for one commit and the CLI suite now covers it.
- `Registry::boards()` always includes the repository you're standing in, listed
  or not — you should never open the board and not see the one you're in.
- Ids restart at 001 in every repository, so a `Card` carries which repo it came
  from; the board shows the name whenever more than one is in view, and a move
  goes through *that* repository's `Board`, not the current one. A drag between
  repositories is refused: the ticket would need a new id, not a rename.
- `load_from`/`save_to` take a path, so tests need no environment mutation. The
  CLI suite sets `XDG_CONFIG_HOME` to a temp dir — `amd repos add` writes to the
  config directory and a test must never touch the user's.

## Completions

- Generated, never checked in: `amd completions <shell>` renders from
  `Cli::command()`, so a new flag is completable the moment it exists. Handled
  before `ensure_board()` — it must work outside a repo, which is exactly when
  people set completion up.
- Value-level completion comes from `crates/amd-cli/src/value.rs`. `TypedValueParser` lets a
  parser advertise `possible_values()` (used by completions and `--help`) while
  `parse_ref()` keeps our own validation and error text — `PossibleValuesParser`
  alone would have replaced "unknown branch type 'x' … set AMD_BRANCH_TYPES"
  with clap's
  wording. `TicketType` advertises the built-ins but accepts any name, since a
  board can add templates.
- `clap`'s `string` feature is on for `Str: From<String>`, which the runtime
  value lists need.
- The script goes to stdout, the install hint to stderr, so a redirect gives a
  clean file. `tests/test.sh` runs `bash -n` over the generated bash script.

## Commands

```bash
cargo test                          # the whole workspace
cargo build -p amd-cli --no-default-features   # the CLI with no windowing stack
bash tests/test.sh
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
bash install.sh [--dir DIR] [--version vX.Y.Z] [--from-source]
```

## Coding style

- Edition 2024, `rust-toolchain.toml` pins **stable**, MSRV 1.88 (let-chains).
  CI gates on `cargo fmt --check` + `cargo clippy -D warnings` + tests.
- Errors: `anyhow` with `.with_context()`; `main` prints `amd: {err:#}` and
  exits 1, matching the old bash `die()`.
- Shell out to `git` rather than linking a git library — behaviour is exactly
  what the user would get typing the command.
- No `unwrap()`/`expect()` on anything user input can reach.
- Keep the CLI surface stable: `tests/test.sh` is the spec, and the assertions
  there are the compatibility contract with v3 boards.
- The shell files still follow the repo's bash rules (see the `shell-scripting`
  skill): quote the literal `"done"` (SC1010), `set -euo pipefail`, and
  shellcheck `--severity=warning` clean.

## Versioning & releasing

Cut releases by tag: `git tag -a vX.Y.Z && git push --tags` triggers
`release.yml`, which builds and attaches `amd-<target>.zip`. Force-move the
major tag (`git tag -f -a vN`) and push it, since `install.sh` and the README
`curl` URL point at `raw…/agile-md/vN`. Invocation model by major (breaking
changes):

- **v1** — script vendored per board (`tasks/task`); board = the script's own dir.
- **v2** — global `task` command; board discovered from the repo root; `task init`.
- **v3** — command renamed `task` → **`amd`**; env `TASK_YES`/`TASKS_DIR` →
  `AMD_YES`/`AMD_DIR`.
- **v4** — bash script → **Rust binary**; tasks rendered from MiniJinja
  templates; `inquire` forms when a value is missing; one flexible ticket whose
  `branch-type` decides whether entering `todo/` makes a branch; a desktop
  board. Two binaries over one library: `amd` and `amdui`. Install downloads a
  prebuilt binary instead of copying a script. The v3 CLI and board layout
  still work; v3 task files simply have no labels, so entering `todo/` on one
  leaves the branch alone. **Within v4 itself**, destination-as-verb dispatch
  (ADR-0006) later retired the original `amd start`/`amd back`/`amd rm`
  vocabulary outright in favour of `todo`/`doing`/`done`/`backlog`/`abandon` —
  no new major version, since the ticket file format didn't change.

## Epics and sprints (`crates/amd-lib/src/group.rs`, ADR-0009)

- A group is a directory under `backlog/` with a `_group.md` describing it.
  **Inside `backlog/` the folder decides** which group a ticket is in; outside
  it there are no group directories, so the `epic` frontmatter key is the only
  carrier. `Board::tasks_in_group` treats the two cases separately — do not
  collapse them.
- A sprint's scope is a query over **every column**, not a directory listing. A
  ticket in `doing/` still belongs to the sprint it was committed to. Counting
  `backlog/` alone made a sprint's points fall as work progressed.
- **A started sprint takes tickets in and lets them out.** It refuses two
  things: an unsized ticket (it would make the total a lie) and an *archive*
  straight out of the sprint (a move out is a dated `git mv`; an archive is
  gitignored and is not). `State::Started` marks a moment, not a lock.
- **Both epics and sprints are optional**, and both stay in scope —
  ticket 032's 2026-09-22 note that dropped sprints (this board was
  human-plus-AI only) was itself reversed 2026-09-25: tickets now live in a
  pushed, git-tracked repository, which makes a real team a near-term case.
  Nothing here changed as a result; sprints were never actually removed
  from the code, only from that ticket's own notes.
- **`Board::empty_group`** moves every ticket a group holds back to the
  loose backlog at once, no precondition — reuses `set_epic(task, None)`
  per ticket rather than a new mechanism, so it inherits the same
  lease/hash checks that setter already has, and skips (rather than
  aborts on) a ticket it can't move. `amd group empty <name>` in the CLI,
  an "empty" button next to a group's "edit" button in `amdui`.
- **WIP limits** (`board_settings::wip_limit`) gate entry into `todo`/
  `doing` only — `Board::move_task` checks the destination column's count
  against the configured limit (env, then `.settings`, same precedence as
  `lease_hours`) before the existing lease check, refusing with
  `WipLimitExceeded` unless `force` is set. Moving a ticket *out* of an
  over-limit column is never blocked, because the check only ever looks at
  where a ticket is going, not where it's leaving from. `move_task`'s
  signature carries `force: bool` now — both its callers (`amd-cli`'s
  `move_and_report`, `amdui`'s `apply()`) pass it through; the GUI always
  passes `false` (no drag-and-drop escape hatch), a blocked drag just sets
  the status line, the same treatment a lease refusal already gets.

## The desktop board (`crates/amd-lib/src/gui/`)

- egui/eframe, the same stack as rustmark, so the two look alike and a ticket
  body could be handed to it. `App::ui` takes a `&mut Ui`, not a context —
  0.36 moved panels onto `Ui`.
- Two ways in, one `gui::run()`: the `amdui` binary and `amd gui`. Neither
  needs a board — both resolve repositories through the registry, so the
  window opens from anywhere.
- Drops are collected during the frame and applied after it, so the UI never
  mutates files mid-draw, and every one goes through `Board::move_task` or
  `Board::set_epic` — the same calls the CLI makes. The GUI must never write a
  task file itself.
- `dnd_drag_source` senses drag only, so opening a ticket needs its own click
  interaction over the card's rect.
- Ordering is an `order` key, ranked fractionally: a card dropped between two
  others takes the midpoint, so one drag rewrites one file. **It is read by the
  board only** — `amd ls` still sorts by id, which is a real inconsistency.
- Errors from a save go to the status line *behind* the modal, so a failure
  reads as "the button does nothing". See ticket 025.
- `BoardApp.identity` (`git::config`, resolved **once** at `load()`) is who
  the badge on a leased card compares against — not re-resolved per frame or
  per card, since egui redraws continuously and `git::config` shells out.
  Dragging a card into Todo or Doing auto-takes a free lease the same way
  `move_and_report` does on the CLI side; column moves and the editor's save
  (`apply_edit`) inherit the actual refusal for free from
  `Task::write_checked`/`Board::move_task` — the GUI only adds the
  auto-take and the badge, not a second enforcement path.
- **`Accent`** (`gui/settings.rs`) is a global colour, not a per-element
  palette editor: an inventory of every `Color32`/`.visuals()` call in
  `gui/mod.rs` found only two genuinely hardcoded colours (status-error
  red, the lease-badge amber, both semantic — left untouched by design);
  everything else already derives from `ui.visuals()`, so overriding
  `Visuals::selection.bg_fill`/`hyperlink_color` via `ctx.all_styles_mut()`
  (both light and dark styles, since `ThemePreference::System` can pick
  either) reaches every one of them for free. `Accent::Default` applies no
  override at all, leaving pre-ticket-039 behaviour exactly as it was.
- **"Add epic"/"Add sprint" render on both views, not just Backlog.**
  Gating them to `View::Backlog` alone made the feature effectively
  invisible — the Board view is the default, so they were never on screen
  unless you already knew to switch tabs. `GroupDraft.repo` and `New
  ticket` both resolve to `self.backlog_repo` regardless of which view is
  showing — Board has no repo selector of its own, so it targets the same
  repo Backlog would.
- **`backlog_repo`'s initial value is looked up, never assumed to be `0`.**
  `home_repo_index()` finds the repo `amdui`/`amd gui` was actually launched
  from by matching `Board::locate()`'s root against `self.repos`. This used
  to be hardcoded to `0` in three places (`BoardApp::load()`'s init, and
  both `View::Board`'s "New ticket"/"Add epic" arms, which read `0` where
  `View::Backlog` read `self.backlog_repo`) — a real bug, not a stylistic
  inconsistency: `Registry::add()` sorts entries alphabetically once a
  repository is registered (the normal case — both binaries auto-register),
  so index `0` is whichever repo's name sorts first, not the one you're
  standing in. `amdui` launched from `oberi-kitchen-tickets` would open
  showing (and creating tickets in) `agile-md-tickets` purely because "a"
  sorts before "o", with no relationship to the current directory at all.

## Conventions

- **Feature branch + PR for every real change** — `main` is not pushed to
  directly (the one exception was the initial full-source-tree restore after
  this repo's history was reset). Workflow-file changes need a
  `workflow`-scoped token.
- Co-authored commits use the bot identity, not the Anthropic no-reply:
  `Co-Authored-By: Claude <309050497+MrDClaudeBot@users.noreply.github.com>`.
