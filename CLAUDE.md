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
- `crates/amd-cli/src/init.rs` — `amd init <project>`: finds or creates
  **two** private GitHub repositories, `<project>-tickets` and
  `<project>-governance` (via `gh`, as siblings under the current
  directory), clones each, scaffolds them, registers both locally, and
  aliases them `tickets`/`governance` (`crates/amd-lib/src/alias.rs`,
  ticket 026) — warning rather than failing if either alias is already
  taken by something else on this machine. Tickets gets the board;
  governance gets `adr/`, `rfc/`, `docs/`, `README.md`, `CHANGELOG.md`, and
  a `.settings/` **directory** (ticket 057 — one small file per key, so a
  changed setting reviews as its own small diff; `.settings/project` is
  the only file `init` writes today). Each repository gets its own branch
  protection tier via `github.rs`'s `protect_main()` — tickets: blocks
  force-push/deletion only; governance: additionally requires one approving
  review — both `enforce_admins: true` so the owner can't silently bypass
  it, neither ever sets `require_last_push_approval` (that combination is
  what previously made a solo pusher's own PRs permanently unmergeable
  elsewhere in this org). **This org's repos are private on the free
  plan**, where GitHub's branch-protection API (both the classic endpoint
  and the newer rulesets one) returns `403 Upgrade to GitHub Pro` for
  *any* payload — `protect_main` treats that as `ProtectionOutcome::Unavailable`,
  not an error, and `init` just warns; review on governance is a human
  convention until the org upgrades or the repos go public, not something
  GitHub enforces yet. **Refuses to run from inside an existing git
  repository** — a guard, not an inference: an earlier version tried
  placing the new repos as siblings of whatever repo you happened to be
  standing in, which was a real source of confusion. `--local-only` is the
  command for "inside a repo" — it skips all of the above (tickets board
  only, no governance sibling, no protection) and just scaffolds here, the
  way bare `amd init` always has.
- `crates/amd-cli/src/github.rs` — `protect_main(full_name, require_review)`:
  the only thing that shells out to `gh api` rather than plain `gh repo`/
  `git`. Needs the branch to exist first (protection is set after the
  first commit is pushed, not before).
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
- `Board::discover()` is `locate()` plus one fallback, used by `ensure_board()`
  (so `amd register` goes through it too, like any other ordinary command —
  it's only the *path* `amd register` defaults to when none is given that
  still resolves via `git::repo_root()`, unchanged, since that specifically
  means "the repo I'm standing in") and the GUI's `BoardApp::load()`. Not
  used by `amd init --local-only`, for the same "repo I'm standing in"
  reason: when the current directory isn't inside a git repository at
  all, it looks at the directory's own immediate children for exactly one
  that's both a git repo and has a board of its own
  (`project_siblings()`) — the layout `amd init <project>` creates, a plain
  parent folder holding `{project}-tickets` and `{project}-governance` as
  siblings. Lets `amd`/`amdui` run from that
  parent directory itself, not only one `cd` deeper. Two or more qualifying
  children is an error naming all of them, never a guess, same as `find`
  on an ambiguous ticket reference.
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
  prompt), `AMD_NO_BRANCH` (never touch branches), `NO_COLOR`/`--plain`
  (plain board), `EDITOR` (for `amd edit`), `AMD_LEASE_HOURS` (lease
  duration, default 8), `AMD_WIP_TODO` (todo's WIP limit, unset = unlimited),
  `AMD_WIP_LIMIT` (doing's WIP limit — unset or unparsable defaults to **1**,
  not unlimited; `0` turns it off), `AMD_COMMIT` (auto-commit-and-push,
  unset = **on**; `0`/`off`/`false` turns it off). **`AMD_TYPES` is dead** —
  it was in `--help` long after nothing read it. **`AMD_NO_REGISTER` is also
  gone** (2026-09-26) — registration is a deliberate act (`amd register`)
  again, not something running in the background to opt out of.
  `AMD_BRANCH_TYPES` and `EDITOR` also fall back to the settings file
  (`branch-types`/`editor`) when unset — the env var wins when both are set
  (`settings::resolved()`). `AMD_LEASE_HOURS`/`AMD_WIP_TODO`/`AMD_WIP_LIMIT`/
  `AMD_COMMIT` follow the same precedence against the *board-local*
  `.settings`'s `lease-hours`/`wip-todo`/`wip-limit`/`commit` keys instead —
  team decisions, not personal ones. `wip-limit` (2026-09-27) replaced an
  earlier `wip-doing` key entirely — see `board_settings::wip_limit`'s doc
  comment for why todo/doing are deliberately asymmetric now, not a naming
  tidy-up.
- **Auto-commit-and-push** (ticket 012, `board_settings::auto_commit`,
  `main.rs`'s `with_auto_commit`) — every ticket-mutating command commits
  (and pushes) its own change, the audit trail a dedicated, board-only
  `{project}-tickets` repository has no other way to accumulate (nothing
  else in it ever changes at the same time a ticket does, so there's no
  natural commit for a move to ride in on). Default **on** — the opposite
  default from every other setting in this file, deliberately: the feature
  is broken until it's active, so off-by-default would ship it in a state
  where the common case needs a setting found first.
  - **The paths committed are computed, never hand-listed.**
    `git::dirty_paths` snapshots the repo before and after the mutation
    runs; `git::changed_paths` diffs the two. This isn't a simplification
    over passing an explicit paths list — it's the fix for a real bug a
    hand-maintained list would have shipped with: `Board::record_id` (via
    `amd new`) also writes `.settings`'s `next-id` counter, so a new ticket
    genuinely touches two files in one operation, and nobody wrote that
    call site expecting to enumerate `.settings` too.
  - **The diff has to be signature-based, not presence-based.** An
    untracked file's `git status` is always `??` regardless of content, so
    a path already dirty in the *before* snapshot that changes again by the
    *after* snapshot is invisible to a plain set difference — hit directly
    on a board created with `amd init --local-only` (which never commits),
    where `.settings` is untracked from the start and a second `amd new`
    needs its *second* bump noticed. `dirty_paths` maps each path to a
    modified-time signature instead of returning a bare set, and
    `changed_paths` compares signatures, not just presence.
  - **`git status`'s default collapses a wholly-untracked directory into
    one line.** Also hit directly on an `--local-only` board: without
    `--untracked-files=all`, a brand new ticket inside a never-committed
    `tasks/` never appears as its own path, because git reports the whole
    directory as a single untracked entry.
  - **`commit_paths` skips `add` for a path that no longer exists.**
    `git mv` (used for a tracked rename) already stages both sides of the
    move in the index by itself; re-`add`ing the now-gone old path
    afterward fails with "did not match any files", since there's nothing
    left for `add` to reconcile. The commit's own pathspec still needs
    both paths, gone or not, to record the whole rename — same reason a
    two-commit rename needs both paths on the second commit, not just the
    first.
  - **`changed_paths` drops a rename hop `git` has already forgotten** —
    found live, against the real `agile-md-tickets` repo, not anticipated:
    a ticket already mid-rename (staged, uncommitted) *before* this
    feature's first use on it, then moved again before either side ever
    landed in a commit, collapses in git's own index straight from the
    original committed path to the final one — the intermediate hop never
    existed in any commit, so it stops appearing in `git status` at all.
    Without also requiring the path to still be a key in the `after`
    snapshot, `changed_paths` kept reporting it as "changed" (dirty in
    `before`, simply absent from `after`), and handing that stale path to
    `git commit` failed outright. No data was ever at risk — the file had
    already moved correctly on disk; only the automatic commit failed,
    caught by the same warn-not-fail policy below.
  - **Best-effort, matching `maybe_take_lease`'s own tone**: a commit or
    push failure (locked signing key, no agent, network down, a moved
    remote) warns to stderr and moves on — never fails the command, never
    rolls back. The board change already succeeded on disk; failing at
    that point would misreport a move that actually worked.
  - **Scope**: every verb in `Board::move_task`/`archive`/`set_epic`/
    `Task::assign`/`take_lease`/`release_lease`/`break_lease`, plus every
    `amd group` command including `empty` (one commit for every ticket it
    moves, not one per ticket — a single logical action). Not in scope:
    `amd register`/`deregister` (the registry, not the board),
    `{project}-governance` (a different repository entirely, PR-reviewed by
    design, never auto-committed).
  - **`amd log`** (`cmd_log`) renders the board's transition history —
    `date time action id title sha` — from `git log --name-status -M`,
    reading paths and rename status only, never the commit message, so it
    works retroactively over history from before this feature existed too.

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

## Several repositories: declared names + local paths, not one shared list

Two cooperating halves, deliberately kept separate — reworked 2026-09-26
after the old single, per-machine registry predated the two-repo topology
and stacked *every* repository ever worked in, across every unrelated
project, into one `amdui` window:

- **Declared** (`board_settings::repos()`/`declare_repo()`, board-local
  `.settings`, git-tracked): the repository *names* a project spans, shared
  with the whole team. One member adding a repository means everyone sees
  it listed on their next pull. Comma-separated, same shape the global
  settings file's `branch-types` already uses — no new persistence
  mechanism.
- **Registered** (`crates/amd-lib/src/registry.rs`, `$XDG_CONFIG_HOME/agile-md/repos`,
  per-machine, unchanged in shape from before): whether a declared name is
  actually cloned on *this* machine, and where. `Registry::find(name)` is
  the only question this file answers now — *which* names are worth asking
  about always comes from the current project's own declared list, never
  from scanning this file's entries wholesale. There is deliberately no
  store of tickets in either half: the markdown is the source of truth, it
  changes under you on every checkout and pull, and an index would be
  stale the moment it was written.
- **`amd register [path]`** (defaults to the repo you're in) does both
  halves at once — declares the name for the project, registers the path
  locally — and is the *only* writer now. Registration is deliberate again:
  `registry::remember()`'s run-on-every-command auto-fill is gone, along
  with `AMD_NO_REGISTER` (nothing left to opt out of).
- **`amd deregister <name>`** removes only the *local* registration. The
  declared list is untouched on purpose — you not having a repository
  cloned right now shouldn't silently un-declare it for the rest of the
  team. It goes back to showing greyed out for you, the same state a
  teammate who's never registered it at all would see.
- **A third, purely local layer sits on top: aliases** (`crates/amd-lib/src/alias.rs`,
  `$XDG_CONFIG_HOME/agile-md/aliases`, ticket 026). A short name — `tickets`
  instead of `agile-md-tickets` — that resolves to a registered repository's
  real declared name, set with `amd alias <name> <alias>`. Deliberately a
  third file, not a second column on the registry's own one-path-per-line
  format (that file's doc comment is explicit it's "a list of paths and
  nothing more") — reuses `settings::Config`'s existing `key = value`
  parser rather than inventing a new one. Refused if the target isn't
  registered, or if the alias is already a *different* repository's own
  real name — an alias and a real name must never be ambiguous with each
  other. **Narrower surface than it might sound**: the only two places that
  ever take a repository name as free-typed input are `amd deregister` and
  `amd alias` itself — `BoardApp::load()`'s own repo-resolution loop
  (`gui/mod.rs`) only ever sees a project's *declared* names (git-tracked,
  never an alias), so it needs no resolution step at all.
- **`BoardApp::load()` (`gui/mod.rs`) cross-references the two.** `Board::discover()`
  finds the current project (a hard error now if none is found — `amdui`
  operates on exactly one project, never a cross-project stack); its own
  board is always `repos[0]`. Each declared name with a registered,
  board-having local entry becomes another `RepoBoard`; each declared name
  *without* one goes into `unregistered` and renders greyed out and inert
  in the `View::Board` repo row — visible, so a teammate who's added a
  repository knows it exists, not silently omitted.
- Ids restart at 001 in every repository, so a `Card` carries which repo it came
  from; the board shows the name whenever more than one is in view, and a move
  goes through *that* repository's `Board`, not the current one. A drag between
  repositories is refused: the ticket would need a new id, not a rename.
- `Registry::load_from`/`save_to` take a path, so tests need no environment
  mutation. The CLI suite sets `XDG_CONFIG_HOME` to a temp dir — `amd
  register` writes to the config directory and a test must never touch the
  user's.

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
  `board_settings::set_wip_limit(column, limit)` is the programmatic
  setter (`amdui`'s Settings modal, and `amd wip-limit <n>` for doing,
  both use it).
  **Todo and doing are deliberately asymmetric (2026-09-27).** Todo:
  absent, unparsable or `0` all mean unlimited, unchanged from when this
  was built. Doing: absent or unparsable now defaults to **1**, not
  unlimited — a queue of more than one "in progress" ticket is treated as
  the thing worth naming a default against — and `0` is stored *literally*
  by `set_wip_limit` rather than collapsed into clearing, specifically so
  "nobody set it" (→ 1) and "somebody set it to 0" (→ off) stay two
  different states on disk. `wip-doing` doesn't exist any more; the key is
  `wip-limit`, and `AMD_WIP_DOING` is `AMD_WIP_LIMIT` — a rename with a
  behaviour change riding along, not a tidy-up, so don't "fix" doing back
  to matching todo's symmetry without re-reading why.
- **`Board::set_epic` only moves the file inside `backlog/`.** Filing a
  `todo`/`doing`/`done` ticket into a group (or clearing it) tags
  `set_epic_meta` in place and leaves the file exactly where it is — column
  and path both unchanged. Before this branch existed, `set_epic` computed
  its destination unconditionally under `backlog/<name>/`, which would have
  silently relocated a `doing`/`done` ticket back into the backlog; nothing
  called it on a non-backlog task until `amdui`'s membership checkboxes
  (below) needed to, so the bug was latent, not yet shipped-and-observed.
- **`Board::delete_group`/`amd group delete <name>`** removes a group's
  `_group.md` and directory outright. Refused (naming the count) if
  `tasks_in_group` finds any ticket still claiming membership, checked
  across every column the same way `empty_group` already does — not just
  the physical folder — since a committed sprint's tickets can sit in
  `doing`/`done` while the backlog folder itself is empty. `empty` it
  first; `delete` does not imply `empty`.
- **Next round of epic/sprint design, not built yet**: sprint backfill, a
  `Closed` state, and burn-up reporting into the documents repo are
  designed in `agile-md-documents`'
  `rfc/0005-sprint-backfill-closing-and-burn-up-reporting.md` — including
  one question it deliberately leaves open (whether a running sprint's
  `todo` is a gate on the existing column or a nested one). Nothing below
  describes any of that; it isn't shipped.

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
- **The Settings modal mixes personal and board-level state, on purpose.**
  Text size/theme/accent are `GuiSettings` — personal, local, saved on
  change. The WIP-limit fields below them are board-level
  (`board_settings::set_wip_limit`, `backlog_repo`'s own `.settings`,
  git-tracked and shared) — loaded fresh into
  `wip_todo_input`/`wip_doing_input` each time the modal opens (same
  load-on-open lifecycle `GroupDraft` uses), saved via a separate
  before/after snapshot comparison from `GuiSettings`'s own, since they
  write to a different file entirely. An empty or unparsable field parses
  to `None`, which both fields tolerate with nothing further to validate —
  but the two no longer *mean* the same thing left blank: `wip_todo_input`
  blank still reads back as unlimited, `wip_doing_input` blank now reads
  back as doing's own default of 1 (see the WIP-limits bullet above).
- **The epic/sprint selector: a toggle, a dropdown, New/Delete — replaces
  the old "Add epic"/"Add sprint" buttons and the old one-lane-per-group
  Backlog layout.** `RepoBoard.selected_kind`/`selected_group` are per-repo
  (each registered repo has its own independent groups, same reason
  `groups` itself is per-repo); switching the Epic/Sprint toggle clears
  `selected_group`, since a name from the other kind's list means nothing
  once the dropdown stops listing it. "New" opens the existing
  `GroupDraft` modal with `kind` taken from the toggle; "Delete" (disabled
  when nothing's selected) calls `delete_group_now`, which — like
  `empty_group_now` beside it — reports an `Err` to the status line rather
  than a dialog. Visible on both views, not just Backlog, and not scoped
  to `View::Backlog` the way the old buttons briefly were by mistake — see
  the still-true reasoning in the next bullet.
- **Selecting a group is a board-wide *soft* filter, not a hard hide, and
  every card gets a membership checkbox.** With a group selected, every
  card in every column — Backlog's now-single flat lane
  (`column_ui(..., Column::Backlog, ...)`, not a per-group lane any more)
  and the Board tab's `todo`/`doing`/`done` — renders a checkbox: checked
  and full-opacity for a current member (`card_in_group`, which reads
  `task.epic` in `backlog/` and `task.epic_meta()` elsewhere, mirroring
  `Board::tasks_in_group`'s own split), unchecked and dimmed
  (`ui.set_opacity(0.5)`) otherwise. Dimmed rather than hidden specifically
  so a `doing`/`done` ticket can still be *added* to the selected group
  from wherever it already sits — a hard hide would leave no checkbox left
  to tick. Toggling calls `Board::set_epic` (safe from any column, per the
  non-backlog branch above) via `BoardApp::toggle_membership`, which
  replaced the old drag-a-card-onto-a-lane `refile` — that mechanism doesn't
  exist any more now that groups don't get their own drop-target lane.
  `group_header_ui` renders the selected group's own name/description/
  points/edit/start/empty controls once, above the list, replacing what
  used to be each lane's own header.
- **The checkbox has to be a sibling of `dnd_drag_source`, not nested
  inside it, and it needs a real label — both confirmed the hard way, live.**
  A checkbox nested inside `dnd_drag_source`'s content closure never got a
  clean click: `dnd_drag_source` senses `Sense::drag()` over its whole
  rect, and the slightest pointer movement during a real click reads as a
  drag starting. Moving the checkbox to a sibling position in the same
  `ui.horizontal` fixed that. A *second*, independent issue surfaced after:
  an empty-label checkbox's native hit box is small and is **not** enlarged
  by wrapping it in `ui.add_sized` — egui sizes a `Checkbox` off
  `ui.spacing().interact_size`, not off an external wrapper, so the extra
  allocated space becomes dead space a click can miss into, hitting
  neither the checkbox nor the card. A short label (`"in"`) fixed it for
  real: the label is part of the same clickable atom as the box, which is
  what actually gives a forgiving, unambiguous target — not a cosmetic
  choice.
- **`GroupDraft.repo` and `New ticket` both resolve to `self.backlog_repo`
  regardless of which view is showing** — Board has no repo selector of
  its own, so it targets the same repo Backlog would.
- **`backlog_repo` is always `0` — and as of the declared/registered
  registration split (2026-09-26), that's no longer a lookup, it's a fact
  by construction.** `repos[0]` is the current project's own board,
  pushed first in `load()` before anything from the declared list; nothing
  after it can end up ahead of it. This is actually the *second* fix to
  this value, worth knowing if `git blame` turns up the first: it used to
  be hardcoded `0` against `Registry::add()`'s alphabetically-sorted,
  cross-project entry list (the normal case, since both binaries
  auto-registered every repo ever touched), which meant `amdui` launched
  from `oberi-kitchen-tickets` could open showing `agile-md-tickets`
  purely because "a" sorts before "o". That got a real fix,
  `home_repo_index()`, matching `Board::discover()`'s root by position —
  correct, but still a lookup against a list `0` had no natural
  relationship to. Once registration stopped being a single cross-project
  list and `repos` started life as "the current project first, then its
  own declared repos", the lookup itself became unnecessary; removed
  along with its tests rather than kept as a no-op.

## Conventions

- **Feature branch + PR for every real change** — `main` is not pushed to
  directly (the one exception was the initial full-source-tree restore after
  this repo's history was reset). Workflow-file changes need a
  `workflow`-scoped token.
- Co-authored commits use the bot identity, not the Anthropic no-reply:
  `Co-Authored-By: Claude <309050497+MrDClaudeBot@users.noreply.github.com>`.
