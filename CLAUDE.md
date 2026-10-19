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

A **cargo workspace: one library, two interfaces** (ADR-0004), so the command
line and the desktop board share one implementation of the board and the
boundary is enforced by the dependency graph rather than by review:

```
crates/amd-lib/       the library (`agile_md`) — everything that touches the board
crates/amd-cli/       → binary `amd`    (clap, inquire, the $EDITOR scratch file)
crates/amd-ui/        → binary `amdui`  (Tauri 2: the window and its commands)
crates/amd-ui-web/    the board's page  (Leptos 0.8, CSR, built to wasm by Trunk)
crates/amd-ui-model/  types and pure rules amd-ui and amd-ui-web share (wasm-safe)
```

The library keeps the crate name `agile_md`, so every `use` in the workspace is
unchanged. The desktop board is described under "The desktop board" below.

The terminal board (`crates/amd-tui/`, tickets 080/081) and the Tauri spike
(`crates/amd-tauri/`, ticket 078) were removed in ticket 082: the desktop
board is being rebuilt on Tauri and Leptos (ticket 084), which makes both
redundant. Their history is in git if it is ever wanted again.

- `crates/amd-lib/src/lib.rs` — the library: `board`, `board_settings`,
  `branch`, `create`, `documents`, `git`, `group`, `project_trust`, `registry`,
  `settings`, `task`, `templates`, `model`, `richtext`, `transition` (and `gui` behind
  the feature, pending phase 4). **No `render`** — drawing is a front end's
  business.
- `crates/amd-lib/src/transition.rs` — moving a ticket the way `amd todo`/
  `doing`/`done` do: the move, the lease on entering `doing` (ticket 156), the
  branch switch on entering `todo`, and the auto-commit (`with_auto_commit`).
  Shared by every front end so a move leaves the board, git history and
  working tree in the same state from any of them. **Nothing here
  prints** — a front end that owns the screen can't have the library writing
  over it — so it returns ordered `Event::Info`/`Event::Warn`s, which the CLI
  prints on stdout/stderr (`report()` in `main.rs`). The desktop board's
  `move_ticket` uses `move_to` (lease and commit) but not `enter_todo`: the
  board never switches branches, because `enter_todo` would switch the
  *tickets* repository's branch (ticket 053). It also holds `create_ticket`/
  `edit_ticket`/`save_text` (what `amd new`, a field edit and the board's
  editor do, with the auto-commit).
- `crates/amd-lib/src/richtext.rs` — the line-oriented body rules with no
  drawing in them: `classify`, `continuation`, `toggle`, `spans`. Any front
  end's line editor follows them (the desktop board edits rich text in the
  page instead, through `amd-ui-model::rich`).
- `crates/amd-lib/src/model.rs` — the front-end-free board model: `Card`, `RepoBoard`, `read_lanes`/`read_groups`, `card_visible`/
  `card_in_group`, `rank_between`, `splice_body`, and `load_repos()` (the
  current project's board plus every declared, registered repository). The
  desktop board reads tickets through it, as any other front end should.
- `crates/amd-lib/src/create.rs` — making a ticket, split into `Draft::prepare` (resolve and
  render, touching nothing) and `Draft::write` (file, child link, backlinks).
  That gap is what lets the CLI put the rendered ticket in `$EDITOR` before
  anything is written, while the desktop board writes what it already has.
- `crates/amd-lib/src/group.rs` — epics and sprints: the folders a backlog is divided into.
- `crates/amd-lib/src/settings.rs` — the shared config file, which keeps keys it does not
  recognise so one writer cannot delete another's.
- `crates/amd-lib/src/board_settings.rs` — the board-local settings file
  (`<repo-root>/.settings`, git-tracked, hidden): the `commit` auto-commit
  toggle and the project's `users` (ticket 088 —
  `Name <email>, …`, git's own identity shape, parsed on `>` so a comma in
  a name survives; email is the identity) live here. **So does every other
  board setting** (ticket 111): `project`, `repos`, `wip-limit`,
  `lease-hours`, `ticket-numbering`, `sync`. Ticket 057 had moved some of
  them into `{project}-documents/.settings/` so they'd be PR-reviewed, but
  a change there only reached the team once a documents PR merged, and the
  copies drifted (agile-md's registered repos sat uncommitted in documents
  while its tickets copy was stale). Now `load`/`save` use only the
  tickets file, and a setting change goes out like a ticket move:
  `amd wip-limit`, `amd register`/`register --remove` and amdui's Settings
  all run through the auto-commit path. `migrate_from_documents` (run by
  the CLI after `catch_up`, and by amdui's `change()`) carries a documents
  `.settings/` across once — `repos` unioned, other keys filling gaps,
  tickets winning a clash — commits it, says so, and is silent after.
  **Once only** (ticket 182): it records `migrated-from-documents = on`
  and never reads the documents folder again, so a repository
  deregistered later isn't re-added from an old `repos` file.
  `documents_root()` survives only to find that old directory: sibling path
  first, then a project-scoped registry lookup, never a machine-wide name.
- `crates/amd-lib/src/current_project.rs` — several projects in one folder
  (ticket 093). The *project directory* is the folder holding a project's
  repositories (a repo's parent, or the current directory outside any repo);
  `.current-project` there (one line, machine-local, never committed) names
  the project `amd` uses. A candidate's project name is its directory minus
  `-tickets`, else its board's declared `project`, else its directory name.
  `pick()` is what `Board::discover()` calls at both sibling-scan sites;
  `keep_existing()` is what `init`/`join` call so adding a second project
  never switches you (it records the one already there as current). The
  CLI side is `crates/amd-cli/src/project.rs` (`amd project`, `amd project
  use [name]`); `amdui` reads `Board.projects` and calls `use_project`, which
  writes the same file and moves the process to the folder so the file —
  not whichever tickets repo it was launched in — decides.
- `crates/amd-lib/src/ideas.rs` — Ideas (ticket 131): developers' ideas
  for apps and improvements, before anyone means to do them. Each is a
  markdown file in the project's **own** documents repository's `ideas/`
  (never the shared one: ideas belong to a project; `ideas::source`),
  pushed straight to main (ADR 0007). Frontmatter `title`, `author`
  (`Name <email>`, left out when anonymous), `created`, `votes` (a JSON/YAML
  flow list of `"Name <email> +2"`, one per person) and `tickets`
  (`<tickets-repo>#<id>`); notes are a `## Notes` list (`- **Name**, date:
  text`, later lines indented). A file without frontmatter is an idea with
  no votes. Votes are clicks: each ±1, kept within ±3 per person (0 drops
  the line); the score is the sum; `sort` is score, then voters, then
  newest. "You" is git's identity in the documents clone (`Person::of`).
  Votes and notes go through `documents::commit_reapplied`: a push
  rejected because origin moved drops just that commit
  (`reset --keep` to the commit before it), fast-forwards, and makes the
  change again on the new file (up to three times), since two people
  voting edit the same line and a rebase replay would conflict.
  `to_ticket` makes a backlog ticket (`source: "idea:<repo>/<path>"`,
  notes naming the idea) and records it on the idea. `to_group` (ticket
  231) makes an empty epic or pending sprint instead, named from the idea
  (or as given), its `_group.md` description `From the idea “T”: ` plus
  the write-up on one line (curly quotes, since the field is in straight
  ones), recorded on the idea as `epics`/`sprints` (`<tickets
  repo>#<name>`, `Idea.epics`/`sprints`). Refused in an
  archived project. CLI: `amd idea [--all]`, `idea new <title>
  [--anonymous] [-e]`, `show`, `vote <ref> <+|++|-|-2…>`, `note`, `ticket [--as
  ticket|epic|sprint] [--name N] [--days D]`
  (`crates/amd-cli/src/idea.rs`, handled before `ensure_board()`). amdui:
  `View::Ideas` (`amd-ui-web/src/ideas.rs`), from the landing page's Ideas
  button: cards best first with your − / + (disabled at ±3), an idea
  opened beside them (write-up via `ideas::write_up`, votes, notes, Add
  note, Make (a backlog ticket, or an epic or sprint for admins only,
  with Name and Days) then Open ticket / Open epic, Open in Documents to
  edit), New idea (rich text, "Put my name on it"). Commands
  `list_ideas`, `read_idea`, `create_idea`, `vote_idea`, `note_idea`,
  `idea_to_ticket`, `idea_to_group` (refused unless
  `boards::admin_access`; the CLI isn't gated, like other admin things).
- `crates/amd-lib/src/roadmap.rs` — Roadmaps (ticket 206): a plan for **one
  board** (repository) of the project, made of milestones. One markdown file
  per roadmap at `<tickets repo>/roadmaps/<board slug>.md`, beside `tasks/`
  and not in it, so the ticket scan never sees it (frontmatter `name`,
  `board`, `goal`, `sprints`; each milestone a `## name` section with
  `- kind:`, `- start:`, `- due:`, `- days-per-week:`, `- duration:` lines
  and its description). The file name is the board's, so a board has at
  most one. A file from before roadmaps had boards (no `board:`) is listed
  for `adopt` by a board and takes no milestones until then.
  `add_milestone(store, target, slug, m)` makes the epic of the same name on
  the **target** board (name only; the description and dates stay on the
  milestone) — or, when the roadmap's `sprints` is on ("Use Sprints"), a
  sprint with the description (one line) and `days` = duration, else start to
  due (`sprint_days`). It checks everything before writing anything.
  amdui commits the roadmap file in the tickets repo and, if the target board
  is another repository, the epic there as a second commit
  (`roadmap-milestone: <name> "<roadmap>"`). **Time** (pure, clock passed
  in): `working_days(from, to, per_week)` counts a week's first
  `per_week` days (Monday first, default 5), both ends counted; a
  milestone's duration is the one given, else start to due in working days;
  `remaining` is working days from today to its due date (negative once
  past); `late` is past due and not all its tickets done, or fewer working
  days left than duration × the share of its tickets not done. A roadmap's
  duration is the sum of its milestones', its remaining runs to the last due
  date, and it is late if any milestone is (`Roadmap::summary`; amdui shows
  red or green). amdui: `View::Roadmaps` (`amd-ui-web/src/roadmap.rs`), from
  the landing page's **Roadmap** button (left of Documents): a grid of
  roadmap buttons by board name with a board dropdown and Create Roadmap on
  the right while a board has none, and a roadmap's own page (goal, Use
  Sprints, milestones); commands `list_roadmaps`, `create_roadmap`,
  `adopt_roadmap`, `save_roadmap`, `rename_roadmap`, `add_milestone`. `rename(store, slug,
  name)` (ticket 210) changes the name and moves the file to the new name's slug
  (`git mv`), refusing an empty name or another roadmap's; milestones, goal and
  board stay. `set_milestone_dates(store, target, slug, name, start, due)`
  (ticket 235) changes a milestone's dates (either may be empty; due before
  start refused); a sprint milestone with no `duration` of its own resizes
  its sprint on the target to start-to-due while it's pending, never once
  started. amdui: each milestone's **Change dates** (`MilestoneRow`, command
  `set_milestone_dates`, `roadmap-dates:` commits, a second one when the
  sprint is on another repository's board). No CLI or MCP tool yet.
- `crates/amd-lib/src/archive.rs` — archiving a project (ticket 157):
  `archived = on` in the tickets `.settings`, auto-committed like any
  setting. An archived project only reads: the CLI's `run()` refuses a
  command `changes_board` calls a change (every `sync_purpose` Write, plus
  the settings commands given a value; `register`/`deregister` and `amd
  import documents` into its own documents check too), and amdui's
  `change()` and `doc_change` refuse (`ensure_writable`). It's off My work,
  work to pick and the issue pull (`my_work::Project.archived`). `amd
  project archive|unarchive [name] [--github]`, Admin's "Archive this
  project", the landing page's "Archived (N)" fold with Unarchive, and a
  banner on the board; `set_project_archived` doesn't go through
  `change()`. `--github` runs `gh repo archive|unarchive --yes` on the
  tickets repository, the project's own documents and each declared
  repository (`github_repos`; never the shared documents). **Order
  matters**, since GitHub refuses pushes to an archived repository:
  archive pushes the setting first, then archives (skipped, with a
  warning, if the push didn't land); unarchive unarchives first, then
  pushes. A `gh` failure is a warning.
- **`amd mcp`: the board for any AI assistant** (ticket 186, instead of a
  built-in chat tied to one provider: not every contributor uses Claude,
  and some use no AI). `crates/amd-lib/src/assistant.rs` holds the tools,
  with no MCP or stdio in it: `tools()` (name, title, description,
  `read_only`, JSON Schema input) and `call(name, args) -> Reply {text,
  data}`. Read: `screen`, `board` (done counted unless asked; 60 cards a
  column), `ticket` (whole file plus `version`, the `ContentHash`),
  `search_tickets`, `my_work`, `search_documents`, `document`,
  `start_page`, `ideas`, `idea`. Change, all through amd-lib with the
  auto-commit: `create_ticket` (`TicketFields`, which gained `priority`),
  `move_ticket` (`transition::move_to`, never `enter_todo`: no branch
  switching), `clone_ticket` (`transition::clone_ticket`), `edit_ticket` (`apply_edit` with unnamed fields kept, plus
  `set_priority`/`set_repository`; `version` makes it refuse a stale
  ticket; no epic, which is `file_ticket`), `assign_ticket` (`@me` is
  git's `user.name`), `add_note` (`**Name, date:** text` at the end of
  `## Notes`), `file_ticket` (`set_epic`). **Never offered:** abandon,
  unlock, settings, branches. Every call opens the project afresh
  (`find_repos` from the directory the AI tool started it in, so a
  `.current-project` switch is followed), runs `project_trust::check` and
  `sync::catch_up` (Read, Lookup or Write), and a Write refuses an archived
  project. A ticket is `repo#id`, or a reference on the project's own
  board, else on the one other board that alone has it.
  `crates/amd-cli/src/mcp.rs` is the wire, by hand (five methods,
  newline-delimited JSON-RPC 2.0, protocol 2025-06-18 back to 2024-11-05):
  `initialize`, `ping`, `tools/list` (annotations: `readOnlyHint`,
  `destructiveHint: false`), `tools/call` (a failing tool is a result with
  `isError`, so the model reads amd's refusal; panics caught). **stdout is
  protocol only**: amd-lib prints nothing but the `git` it runs inherits
  stdio, so `take_stdio` dups 0 and 1 for the protocol and points the
  process's own at `/dev/null` and stderr; `AMD_NO_INPUT` and
  `GIT_TERMINAL_PROMPT=0` are set. `amd mcp setup [claude|vscode|cursor|
  codex|gemini]` prints how to connect each; it writes no one's config.
  **What's on screen** (`crates/amd-lib/src/screen.rs`): amdui's
  `set_screen` writes `$XDG_STATE_HOME/agile-md/screen/<repo>-<fnv1a of
  the path>.json` (pid, time, project, and the page's `ScreenView`:
  screen, open ticket, document, idea, epic column, search, repository
  and assignee filters; names and paths only), from `report_screen` in
  `app.rs`, a `Memo` so it's sent only on change (`ctx.idea` moved into
  `Ctx` for it). FNV, not the standard hasher, so amdui and amd built
  apart agree. The window's `Destroyed` event removes it; a killed amdui
  leaves it, and `read` ignores a note whose pid isn't running (`kill
  -0`). Preferences → AI command shows the Claude Code command and points
  at `amd mcp setup`.
- `crates/amd-lib/src/project_trust.rs` — a per-machine, trust-on-first-use
  check that a board's declared `project` hasn't changed unexpectedly since
  `amd` last saw it.
- `crates/amd-lib/src/update.rs` — is there a newer release (ticket 190,
  the detecting half of in-app updates; applying is 191, the Update / Later
  banner 192, compatibility and `min_version` 193). amd-lib has no HTTP
  client, so GitHub is a `ReleaseSource` the caller supplies (the front
  ends use `ureq`; tests use a fake). `parse_releases` reads the releases
  API answer, dropping drafts, pre-releases and tags that aren't
  `major.minor.patch`; `newest_above` gives `UpdateAvailable {version,
  notes_url, behind}`, `behind` being how many releases the build is
  missing so an update can catch up one at a time. `check(source, current,
  now, state_file)` asks GitHub at most once a day (an hour after a
  failure), otherwise answers from the remembered list; a failed check is
  silent. `snooze` is "Later": it hides *that version* for 24 hours, so a
  still newer release shows at once. State is `$XDG_STATE_HOME/agile-md/
  update.json`, machine-local, never in a repository; a missing or corrupt
  file is a fresh start. `update-check = off` / `AMD_UPDATE_CHECK=0` turns
  the check off (`enabled`). The clock is passed in. **The bar** (ticket
  192): `crates/amd-ui/src/updates.rs` is the `ureq` `ReleaseSource`
  (GitHub's releases API, 10 s timeout) and `status()`/`later()`; commands
  `update_status` (`amd_ui_model::UpdateInfo {version, notes_url, behind,
  current}`) and `snooze_update`. The page's `update_bar.rs` `UpdateBar`
  sits above the header on every view, asks on start and every 6 hours,
  and is **orange**: the accents are only indigo, teal and slate (rose and
  amber were dropped, a saved one reads back as indigo), so nothing else
  is orange. The required variant (no Later) is 193.
- `crates/amd-lib/src/update_apply.rs` — **applying an update** (ticket
  191). `install_dir(exe)`: only a folder whose `amd-uninstall` says
  `# agile-md install: release` (install.sh now writes `release` or
  `source`, `METHOD`) is `Managed`; anything else is `Unmanaged {why,
  reinstall}` (reinstall: the installer command is the fix; not for a
  source build). `apply(dir, current, releases, fetch, progress)` goes
  through `steps` (newer releases, oldest first), one at a time, stopping
  at the first failure (`Applied {reached, failed}`): the asset
  `amd-<target>.zip` (`target()` from ARCH/OS), its SHA-256 checked
  against GitHub's `digest` (`update::Asset`, parsed by `parse_releases`;
  every release file has one), `amd` (required) and `amdui` unzipped to
  `.<bin>.new-<pid>` beside the live ones, each run with `--version`
  (must name the version), then the live one renamed to `<bin>.previous`
  and the new one renamed in (put back on failure). Nothing else is
  written. `rollback(dir)`. `Fetch` is the front end's (`ureq`); deps
  `zip` (deflate via zlib-rs) and `sha2`. CLI `amd update [--check]
  [--rollback]` (`crates/amd-cli/src/update.rs`, before `ensure_board`);
  amdui's `apply_update` (`amd_ui_model::UpdateResult`) and `restart_app`
  (`AppHandle::restart`), the bar's Update (refuses while the editor is
  open; the modal's backdrop already covers the bar). Both read
  `AMD_UPDATE_RELEASES=file://…` and `file://` downloads, which
  `tests/test.sh` uses with a pretend 9.9.9.
- `crates/amd-lib/src/renumber.rs` — `amd renumber [--keep T] [--apply]`
  (ticket 146): tickets sharing a number (`local` numbering can clash when
  clones create apart). The first committed keeps it, judged by commit
  order of the first commit adding that file name anywhere on the board
  (not `git log --follow`, which traces template-alike tickets back to the
  first ticket); `--keep` overrides. Each other one gets the next number:
  `git mv` plus its `id`, and `record_id`. Full `[[stem]]` links are
  rewritten; a bare number in `parent`/`related` only when the moved
  ticket's own `## Children`/`related` proves which ticket it meant, else
  reported as ambiguous and left. Without `--apply` it only shows the plan.
- `crates/amd-lib/src/stats.rs` — statistics (ticket 233), read from git
  history when asked, never stored (RFC 0005). `events(board, since)`:
  `git log -z --name-status -M` over the board, a ticket path being
  `<column>/…/NNN-*.md`: an `A` is raised, a `D` closed (abandoned or
  deleted), an `R` between column folders a move (done-kind judged by the
  board's workflow, a removed column by the name `done`). `counts(boards,
  days, now)` sums raised, done (a move into a done kind from another) and
  closed, and per week (the last ending now). `burns(board, repo, now)`:
  each started sprint, its start the first commit whose `_group.md` says
  `started: "started"` (`sprint_started`, `git log -S`), its scope the
  points of its tickets now (a started sprint takes nothing more), and
  `done_by_day` from when each ticket now done last reached done, up to
  today or its last day. `period()`/`set_period` is `ui.stats-days` (7, 30
  or 90; 30 default). CLI `amd stats [--days N]` (counts, weeks, and a
  text burn-up/burn-down a sprint). amdui: the landing page's Projects
  heading has the 7/30/90 switch (`Home.stats_days`, `set_stats_days`);
  each project row loads `project_stats(root, days)` (its boards via
  `my_work::boards_of`) and shows the counts, a bar pair a week, and per
  running sprint an SVG burn-up (dashed scope) and burn-down (dashed
  ideal) (`ProjectStatsView`, `WeekBars`, `BurnCharts` in `home.rs`).
- `crates/amd-lib/src/my_work.rs` — My work (ticket 020): every project
  registered on this machine (`Registry` entries with a board, scratch
  boards under `std::env::temp_dir()` skipped, named by
  `current_project::project_name`), each with its declared repositories'
  boards (`boards_of`, as `load_repos` does for one project); hidden ones
  in your own config (`ui.hidden-projects`, by path). `my_work(email,
  name, …)` gathers todo and doing tickets assigned to you, as git's
  identity in each board's repository (else `email`/`name`): names from each
  project's users list by email, git's `user.name` only where the email
  isn't listed (else one person's git name would match another's tickets).
  Sorted by priority then oldest (`Sort::{Priority, Age, Points}`). Read
  on demand, no cache, no fetching. `amd mine` (CLI, works from anywhere)
  prints it; the desktop board's landing page shows it. **With nothing
  to do** (ticket 160), `to_pick` offers work from every project, hidden
  ones included, never a draft or a ticket leased by someone else
  (`pickable`). With `ui.include-unassigned` on (the default; My work's
  checkbox, `amd mine --unassigned`/`--no-unassigned`): unassigned todo
  and doing tickets first, by priority; failing those, backlog tickets
  unassigned or yours. Off: only backlog tickets assigned to you. From
  the backlog, only the top priority tier there is (`top_tier`: all high,
  else all medium, else all low), oldest first. TRIAGE tickets count. `show_projects_of` takes a hidden project
  with work to pick off `ui.hidden-projects` for good; `amd mine` and
  amdui's `home` both call it. `WorkItem.folder` carries a backlog
  ticket's epic folder so the page can open it. `--json` still prints
  only My work.
- `crates/amd-lib/src/recover.rs` — `amd recover` (ticket 144): tickets
  removed from the board, found with `git log --diff-filter=D -M` (so a
  move between columns isn't a removal), each read back from its removing
  commit's parent. Listed with who removed it and when; "abandoned" when the
  commit says `abandon` or this machine's archive holds it, else "deleted".
  Only tickets not on the board now. `restore` writes one into the loose
  backlog exactly as it was, keeping its id, and refuses if that number is
  on the board. The CLI filters with `--by` and `--since` and restores
  `<id>...` or `--all` in one auto-commit. A force push that rewrote the
  remote's history is out of its reach (that's 145 and branch protection).
- **`amd import` (ticket 173): stand where the things go, name where they
  come from.** Never overwrites; what's left out is printed and appended to
  `import.log` in the folder it ran from (`import_log.rs`: dated sections,
  the name added to `.git/info/exclude`, so never committed).
- `crates/amd-lib/src/import.rs` — `amd import tickets <PATH>` (or a bare
  path), run from the destination board's `tasks/` directory (`cmd_import`
  guards that, and that `<PATH>` isn't this board and has something to
  import: `import::looks_importable`, at least one standard column folder
  or tickets loose in it, ticket 251). A missing column folder is taken as
  empty (`ImportSummary.missing_columns`), and `NNN-*.md` lying directly in
  the source folder go to the backlog (`loose_tickets`, `ImportSummary.loose`);
  the CLI says both. Reads the source as a read-only `Board` (`Board::at`) and copies
  each ticket in — status kept, id renumbered against the destination's
  own counter, `parent`/`related` remapped when both ends are part of the
  same batch. A ticket whose title is already on the board is **skipped
  and reported**, not imported (ticket 173; it used to be tagged
  DUPLICATE); same-titled tickets within the batch are both imported,
  tagged `DUPLICATE`. `archive/` isn't a destination. A ticket that fails
  to parse is skipped and reported. One auto-commit (`import: tickets`).
  `--dry-run` computes everything without writing or spending an id.
- `crates/amd-lib/src/import_documents.rs` — `amd import documents
  <PATH> [--dry-run]` (tickets 079, 170, 173), run from the folder of a
  documents repository (a repo named `*-documents`, or the project's
  `shared-documents`) the files should go into: `cmd_import_documents`
  passes that folder, relative to the repo root, as `plan`'s `into`.
  `plan` mirrors the tree: markdown, plus files a document links to or
  shows inside the tree (a link scan, not a parser), hidden files skipped,
  symlinks not followed; an existing destination is a clash, skipped and
  logged, never overwritten. The source can't be inside the destination
  repository, or hold it. `apply` refuses a clone with uncommitted tracked
  changes, a detached HEAD, or a branch other than `origin/HEAD`'s, then
  (ADR 0007: straight to main) `documents::catch_up` (fetch, fast-forward,
  refuse diverged), commits, and `documents::push_or_replay` (one
  `rebase @{u}` replay); a failed push keeps the commit and warns. Handled
  before `ensure_board()`. `--into` and `--project` are gone.
- `crates/amd-lib/src/import_issues.rs` — GitHub issues, **pulled in
  automatically** (ticket 173): amdui's `pull_issues`, called by the
  landing page once it shows, does every project on this machine whose
  `import-issues` setting is on (the default; `amd issues on|off`, Admin's
  "GitHub issues" switch), at most every five minutes per project
  (`AppState.issues_pulled`), GitHub asked **without** amdui's write
  lock (ticket 181: holding it through every project's fetch made each
  top-bar button wait up to 15 s on a cold start), and only a project with
  something new takes the lock to catch up, plan again, write and commit;
  silent when there's nothing, no `gh` or no network. `amd issues
  [--repo owner/repo] [--label L] [--dry-run]` checks now (`amd import
  issues` points there); Admin keeps Preview and Check now. Open issues
  from `gh issue list --json` go into the loose backlog through `Draft`
  (template, `created-by`, priority and the counter as in `amd new`),
  tagged `TRIAGE` plus each label the project lists (the others are named
  in the notes), with `source: "github:owner/repo#N"`. `targets`:
  `owner/repo`, or a declared name; none given, every declared repository
  on GitHub — from its clone, else (173) from the tickets origin with the
  name swapped in (`clone::sibling_url`, `git::url_identity` resolving SSH
  aliases) — else the board's own repository. **An issue is imported
  once, ever**: `imported_sources` reads every `source:` line ever added in
  the board's history (`git log -p -G`) plus the files on disk, archive
  included. `plan_with` takes the fetch as a closure, so it's tested
  without GitHub; `tests/test.sh` puts a fake `gh` on `PATH`. Tickets
  are high: every TRIAGE ticket is (ticket 248; the `triage-priority`
  setting is retired, `amd triage-priority` hidden and only says so).
- `crates/amd-cli/src/main.rs` — the `amd` binary: clap CLI and command handlers; it also owns
  `form.rs`, `value.rs`, `completions.rs` and `init.rs`, which are CLI-only.
- `crates/amd-cli/src/init.rs` — `amd init <project>`: finds or creates
  **two** private GitHub repositories, `<project>-tickets` and
  `<project>-documents` (via `gh`, as siblings under the current
  directory), clones each, scaffolds them and registers both locally.
  Tickets gets the board;
  documents gets `adr/adr/`, `adr/rfc/`, `docs/`, `ideas/`, `README.md` and `CHANGELOG.md`
  — no settings (ticket 111: they all live in the tickets `.settings`). Both
  repositories get **the same** branch protection via `github.rs`'s
  `protect_main()`: force-push and deletion of main blocked, direct pushes
  untouched, `enforce_admins: true` so the owner can't silently bypass it,
  never `require_last_push_approval` (that combination is what previously
  made a solo pusher's own PRs permanently unmergeable elsewhere in this
  org). **Documents used to also require one approving review; ticket 174
  dropped it**, since ADR 0007 pushes documents straight to main and a
  review rule would reject every such push where GitHub enforces it. **This org's repos are private on the free
  plan**, where GitHub's branch-protection API (both the classic endpoint
  and the newer rulesets one) returns `403 Upgrade to GitHub Pro` for
  *any* payload — `protect_main` treats that as `ProtectionOutcome::Unavailable`,
  not an error, and `init` just warns. **Refuses to run from inside an existing git
  repository** — a guard, not an inference: an earlier version tried
  placing the new repos as siblings of whatever repo you happened to be
  standing in, which was a real source of confusion. `--local-only` is the
  command for "inside a repo" — it skips all of the above (tickets board
  only, no documents sibling, no protection) and just scaffolds here, the
  way bare `amd init` always has.
  **Both forms record whoever ran it as a user** (ticket 090,
  `add_first_user`): git's `user.name <user.email>` (`User::from_git`) into
  the tickets `.settings`'s `users`. A brand-new repository's initial
  commit carries it; an existing one (re-running `init` is how a project
  made before 090 gets its first user) gets its own `.settings`-only commit,
  pushed when there's an `origin` (`git::commit_paths` vs
  `commit_paths_locally`). **Before writing, an existing clone is caught
  up** (ticket 094, `ready_to_record`): one with uncommitted edits to
  tracked files is left alone, otherwise `sync::catch_up(Write)`
  fast-forwards it (or pushes its own earlier commits); a diverged clone
  is refused. Either way it's skipped with a warning to fix it and re-run,
  never reset or stashed. A rejected push only warns — never `join`'s
  reset-and-retry, which is safe only in a seconds-old clone, and this may
  be someone's real working copy. No git email warns too; `init` still
  succeeds. `--local-only` records it without committing.
- `crates/amd-cli/src/join.rs` — `amd join <project> [--org]` (ticket 088):
  everyone but the project's creator. Clones `<project>-tickets` and
  `<project>-documents` as siblings with **plain `git clone`**, never `gh`
  — the user's SSH key is the only credential; nothing is created and no
  protection is touched. URL is `{AMD_GIT_BASE}{owner}/{project}-{kind}.git`,
  `AMD_GIT_BASE` defaulting to `git@github.com:` (the test suite points it
  at local bare repos). Refuses inside a git repo, and if either directory
  already exists in any form. Checks the tickets clone has a board and a
  matching `project`, adds git's `user.name`/`user.email` to `users`,
  commits `join: Name <email>` and pushes — **always**, regardless of
  `AMD_COMMIT`. A rejected push is retried (up to 5 attempts, jittered
  backoff) by `fetch` + `reset --hard origin/<branch>` + re-applying the
  change: safe only because the clone is seconds old and its one local
  commit is ours — don't reuse that pattern on a clone with real work in
  it. Any failure before the push lands removes the directories that run
  created, so a retry starts clean. Reuses `init`'s `owner_for` and
  `register`.
- `crates/amd-cli/src/github.rs` — `protect_main(full_name, require_review)`:
  the only thing that shells out to `gh api` rather than plain `gh repo`/
  `git`. Needs the branch to exist first (protection is set after the
  first commit is pushed, not before).
- `crates/amd-ui/` — the `amdui` binary; see "The desktop board" below.
  `amd gui` launches it (`cmd_gui`: the `amdui` beside `amd`, else on
  `PATH`), so `amd` carries no window or web stack at all.
- `crates/amd-lib/src/board.rs` — board discovery (`<repo-root>/${AMD_DIR:-tasks}`), the
  `Column` enum, task lookup, moves, `create` (scaffolding — the interactive
  "create one?" prompt itself is `ensure_board()`, CLI-owned).
- `crates/amd-lib/src/task.rs` — `Task` (path, column, id, stem), frontmatter `meta` lookup,
  `slugify` (transliterates via `deunicode`, so a title in any script still
  makes a plain-ASCII filename and branch; symbols stay separators so an emoji
  doesn't become a word). `points` is fixed sprint-planning Fibonacci
  (`VALID_POINTS`: `1 2 3 5 8 13`), enforced once, in `validate_points`,
  called from both write paths (`set_points`, the CLI's `amd set points`;
  `apply_edit`, the combined field save; `set_text`, the desktop board's
  whole-ticket save) so none can produce an invalid value — empty always
  allowed, clears the field. Not configurable: nobody asked for a board to
  pick its own scale, and a fixed list is what makes the board's points
  field a dropdown (`amd_ui_model::ticket::VALID_POINTS`) rather than free
  text.
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
  command with `clap_complete`; `$SHELL` detection and install hints. **Epic
  names and added columns** (ticket 028) are completed at TAB time: bash, zsh
  and fish scripts get a hook (`with_destinations`, patched against lines
  clap_complete writes) that runs `amd __destinations`, so a new epic needs
  no shell reload. `__destinations` is caught in `run()` before clap parses,
  not a hidden subcommand, since clap_complete lists hidden ones in every
  script; it prints nothing without a board, never fetches or prompts, and
  leaves out command names and names with spaces.
- `crates/amd-cli/src/value.rs` — `TypedValueParser`s that advertise their possible values, so
  completions and `--help` list them while validation stays ours.
- `crates/amd-lib/src/git.rs` — thin wrappers over the `git` CLI: repo
  discovery and file state (`repo_root`, `is_tracked`, `mv`, `config`),
  branches (`current_branch`, `branch_exists`, `switch_branch`,
  `commits_ahead`, `branch_pushed`, `delete_branch`, `set_head_branch`), and
  a small set for `amd init`'s tickets-repo bootstrap (`remote_url`,
  `commit_all`, `push`) and `amd join`'s clone-and-retry (`clone`, `fetch`,
  `reset_hard`).
- `crates/amd-lib/templates/*.md.jinja` — the built-in templates, `include_str!`'d into the
  binary.
- `install.sh` — downloads the prebuilt `amd-<target>.zip` from the GitHub
  release, or builds from source in a clone (`--from-source`, `--dir`,
  `--version`). Keep the two paths in sync. **The curl one-liner is the
  install users get** (ticket 238: the README and the website show only
  it; building from source is in CONTRIBUTING.md). `check_system` runs
  first and refuses, saying why, on Windows (pointing at WSL), another OS,
  another processor, or git older than 2.29; a downloaded `amd` that won't
  run (an old glibc) is reported as such and exits 1. A clone is
  `in_clone` (`crates/amd-cli/Cargo.toml` beside the script), never the
  current directory: piped from curl there's no script file, and a user
  standing in their own Rust project used to get a source build attempt.
  A failed download only falls back to a source build in a clone.
  **Uninstalling** (ticket 242): every install writes `amd-uninstall` into
  the same folder, generated by `uninstaller BIN DATA` with the install's
  paths baked in; it removes amd, amdui, the launcher and icons and
  itself, notes a copy still running (by `/proc/*/exe`, never by name),
  then asks on `/dev/tty` whether to remove `~/.config/agile-md`,
  `~/.local/state/agile-md` and `~/.local/share/agile-md` (warning when
  `recovered/` holds tickets); `--all` removes them, `--yes` never asks.
  `install.sh --uninstall [--all] [--yes]` generates and runs the same
  script, finding the folder from `amd` on the PATH unless `--dir`. Both paths install `amdui` as well,
  guarded by `if [ -f ... ]` — `--version` can pin a release made before the
  zip carried both. Every binary goes in through `replace_bin` (ticket 098): copy beside
  it, then `mv` over it, never `cp` onto it — Linux refuses to write into a
  running executable ("Text file busy"), which left an install half done. A
  running one (found via `/proc/*/exe`) gets a "restart it" note.
  **On Linux it also installs amdui's launcher** (`install_launcher`,
  ticket 073): `amdui.desktop` and the icons (`app-icon.svg` plus the 32,
  128 and 256 px PNGs from `crates/amd-ui/icons/`) into
  `${XDG_DATA_HOME:-~/.local/share}`. GNOME's dock and top bar take an app's
  icon from its launcher entry, matched by app id, never from the window,
  so without it amdui shows a generic icon. The app id is `amdui`: Tauri
  leaves GTK's as the program name unless `app.enableGTKAppId` is set,
  which would also make GTK route a second `amdui` to the first process.
  The icons come from the clone, or for a release install from the
  repository at the installed version's tag (`fetch_icons`).
- `tests/test.sh` — assert-based end-to-end suite; builds with cargo and spins
  up temp git repos. Unit tests live next to the code in `#[cfg(test)] mod tests`.
- `.github/workflows/ci.yml` — fmt + clippy + cargo test + tests/test.sh, and
  shellcheck for the shell files.
- `.github/dependabot.yml` and the **Dependabot gate** (tickets 246,
  247; `.github/workflows/dependabot-gate.yml`, `scripts/dependabot-gate.sh`):
  weekly GitHub Actions and Cargo updates (Cargo's minor and patch grouped),
  each only once a release is 7 days old (`cooldown`; security updates
  don't wait). The gate reads each PR's updates from Dependabot's
  `updated-dependencies` commit metadata and decides: **ready** (every
  update a patch, or a minor of a 1.x+ version; no unwithdrawn advisory in
  GitHub's database for `name@new-version`, ecosystem rust/actions; CI's
  `rust` and `shell` passed) is labelled, auto-merged, and approved as the
  owner of `DEPENDABOT_APPROVER_TOKEN` (MrDClaudeBot, a code owner, since
  main needs one); **held: advisory** gets a comment and is looked at again
  by the weekly run; **needs a person** (a major, a 0.x minor, failed
  checks, or superseded by a newer open PR for the same dependency) gets an
  issue marked `<!-- dependabot-pr: N -->`, closed when the PR closes, which
  reaches the board as a TRIAGE ticket through amd's issue import. Runs
  when CI completes on a Dependabot branch (assess only: Dependabot-started
  runs can't read Actions secrets) and weekly on Mondays or by hand
  (apply); never on `pull_request`, which listed its jobs as skipped
  checks on every PR (ticket 250). Closing a PR's issue is
  `dependabot-gate-closed.yml`, on `pull_request: closed` only.
  `DRY_RUN=1` prints instead of acting.
- `.github/workflows/release.yml` — on a `vX.Y.Z` tag, builds `amd` and
  `amdui` for Linux and macOS, x86_64 and aarch64 (ticket 085): WebKitGTK on
  Linux, the wasm target and Trunk, `trunk build`, then `cargo build -p
  amd-cli -p amd-ui`, a `--version` smoke test, and `amd-<target>.zip`
  holding both. A `release` job then creates the GitHub Release (`gh release
  create --verify-tag --generate-notes`) and attaches every zip, so no
  release is made by hand; a platform whose build failed is left out with a
  warning. amd's own workflow, not `lite-actions/rust-release`, because only
  amdui needs the Tauri steps (decided 2026-10-08). `workflow_dispatch` is a
  dry run: artifacts only, no release. macOS x86_64 is cross-compiled on the
  Apple-silicon runner; the binaries aren't signed or notarised.
- `docs/adr/` — architecture decision records: why the tool is shaped as it
  is, and what each decision costs. `docs/workspace-split.md` is the plan for
  the amd-lib / amd-cli / amd-ui split (ADR-0004).
- `scripts/demo-board.sh [DIR]` — builds a fictional "Acme" project for
  screenshots (ticket 147) through the `amd` CLI itself: tickets, code
  repositories, users, tags, two epics, a started sprint, a draft, every
  column, a documents repository and a shared handbook, ideas with votes and
  notes, a roadmap on track, locks only in doing, creation dates spread back. Own
  `XDG_CONFIG_HOME` in `DIR/config`; refuses a DIR it didn't make
  (`.agile-md-demo` marker). Shellchecked in CI.
- `SECURITY.md` (ticket 245): fixes in the latest 4.x only, reports
  privately through the Security tab's advisories, acknowledged within 7
  days; covers amd, `amd mcp`, amdui, `install.sh`/`amd-uninstall` and the
  release builds, not people's repositories or third-party tools.
- `README.md`, `LICENSE` (MIT). The README is the pitch, features,
  install (the curl line) and a quick start; **how to use everything is in
  `docs/user-guide.md`** (ticket 239), and building from source in
  `CONTRIBUTING.md`. A user-visible change updates the guide (and the
  README's feature list if it's a feature).
- `pages/` — the website (tickets 205, 211), published to GitHub Pages by
  `.github/workflows/pages.yml` (only on a change under `pages/` on main, or
  by hand; the repository's Pages source must be "GitHub Actions"). Plain
  HTML and CSS, no build: `index.html`, `style.css`, `screenshots/` (from
  the demo board, built at `/tmp/acme` so no home path shows in amdui's
  header, at 1600 px wide) and `logo/`. **The logo** (ticket 194) is
  `pages/logo/agile-md-light.svg` and `-dark.svg` (the dark one's "Agile-",
  "MD", tagline and bulb base lightened for dark themes), from the
  designer's source vector with the text turned into outlines (`inkscape
  --export-text-to-path`), so it doesn't depend on DejaVu Sans; the README
  and the site pick one with `<picture>`. `crates/amd-ui/app-icon.svg` is
  the logo's mark (board and git diamond, no words), copied to
  `pages/logo/icon.svg` as the favicon; after changing it, `cargo tauri
  icon app-icon.svg` in `crates/amd-ui` regenerates `icons/` (delete the
  `android/`, `ios/` and `64x64.png` it also makes: nothing uses them). The
  feature list is in three places that should agree: the README's
  Features, the site, and agile-md-documents' `docs/FEATURES.md`.
- `Board::locate()` resolves the board as `$(git rev-parse --show-toplevel)/${AMD_DIR:-tasks}`,
  so `amd` works from any subdirectory; it errors if not inside a git repo.
- `Board::discover()` is `locate()` plus **two** fallbacks (ticket 061 added
  the second), used by `ensure_board()` (so `amd register` goes through it
  too, like any other ordinary command — it's only the *path* `amd
  register` defaults to when none is given that still resolves via
  `git::repo_root()`, unchanged, since that specifically means "the repo
  I'm standing in") and the desktop board's `model::load_repos()`. Not used by `amd init
  --local-only`, for the same "repo I'm standing in" reason.
  - **Outside any git repository at all**: looks at the current
    directory's own immediate children for exactly one that's both a git
    repo and has a board of its own (`project_siblings()`) — the layout
    `amd init <project>` creates, a plain parent folder holding
    `{project}-tickets` and `{project}-documents` as siblings. Lets
    `amd`/`amdui` run from that parent directory itself, not only one `cd`
    deeper. Two or more qualifying children **errors**, naming all of
    them, never a guess, same as `find` on an ambiguous ticket reference —
    there's no sane fallback here, `locate()` already failed outright.
    `.current-project` (ticket 093) resolves it when set, and the error
    names `amd project use`.
  - **Inside a git repository that has no board of its own** (ticket 061 —
    `locate()` alone "succeeds" here regardless, since it never checks
    `tasks/` actually exists, which is exactly what stopped this from ever
    running before): checks *that repository's own parent* the same way,
    so standing inside the **code** repo — a sibling of the tickets repo,
    not an ancestor of it — finds the real board next door instead of
    treating the code repo as an empty one of its own. Two or more
    qualifying children here does **not** error, unlike the case above —
    it falls back to `locate()`'s own (possibly nonexistent) answer
    silently, same as finding none at all. A repository's parent is far
    more likely to hold unrelated other repositories than a directory
    purpose-built by `amd init` is — confirmed the hard way: `tests/test.sh`'s
    own scratch boards, all siblings under one shared temp root, made an
    unconditional ambiguity error here break unrelated, pre-existing tests
    that create a fresh repo and expect today's plain "no board yet"
    behaviour. Only a genuinely unambiguous single sibling redirects.
    **Except** (ticket 093): `.current-project` decides when it's set, and
    two or more `*-tickets` siblings with no choice made is the
    plain-directory case's error after all — that's the `init`/`join`
    layout, a folder of projects, where a silent "no board" hid the problem
    (it silently lost the agile-md board from inside `amd/` as soon as a
    second project was initialised beside it). Unrelated scratch boards
    still fall back quietly.
- Tasks are `NNN-slug.md`. `Board::next_id()` takes whichever is higher of the
  counter (its own file, `<repo-root>/.next-id`, ticket 150, so `.settings`
  holds only settings and a rule limiting who may change it (151) never
  stops anyone creating tickets; a board from before still has a `next-id`
  key in `.settings`, or the oldest a `<board>/.next-id` file, and the
  first time a number is recorded the highest of them moves into the
  counter file and the old places are cleared — in `record_id`
  (`migrate_counter`), never while `next_id` works a number out, which
  runs before auto-commit's snapshot and on dry runs, so the `.settings`
  change was never committed (ticket 164)) and one past the highest id
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
- **Columns have kinds** (ticket 223, `crates/amd-lib/src/workflow.rs`):
  `board::Column` is a small `Copy` struct, a folder (interned, so a
  handful of names are leaked once) and a `board::Kind` (`Backlog`,
  `Todo`, `Doing`, `Done`). The four standard columns keep the old enum's
  names as constants (`Column::Backlog` …, `Column::STANDARD`), so they
  read as before. A board's columns, in order and labelled, are its
  `Workflow` (`Board::workflow()`): `columns()`, `position`, `next_to`
  (one column at a time), `first(kind)`, `of_kind`, `find` (folder or
  label, ignoring case), `landing()` (the standard backlog). **Rules ask
  the kind, never the name**: the lease is taken entering any doing-kind
  column and released leaving them (`transition::move_to`); the WIP limit
  belongs to doing-kind columns (`wip-limit` for the standard doing,
  `wip-limit-<folder>` for another); done waits on entering a done-kind
  column and any done-kind ticket holds nothing back (links, requires,
  auto priority, the tree, roadmaps); a draft can't leave the backlog
  kinds; a started sprint's tickets only move on to a todo-kind column;
  `home_group` runs coming back from a todo-kind column; My work covers
  todo and doing kinds and `to_pick` the backlog kinds. Epics and sprints
  still live only in the standard backlog folder, and new tickets land
  there. `Workflow::parse` reads and checks the written form
  (`Label=folder:kind`, comma separated: every kind present, folders
  plain and unique, not `archive`/`templates`, the four standard folders
  kept with their kinds). **A team's own columns** (ticket 224):
  `Board::workflow()` reads its board's `.settings` `columns` (cached by
  the file's mtime and length; an unreadable value is the standard four,
  with `workflow_problem()` saying why); `board_settings::set_columns`
  leaves no key for the standard four. `crates/amd-lib/src/columns.rs`
  changes it: `add` (folder from `slugify(label)`, a `.gitkeep`; refuses
  a name a column, an epic or `abandon` answers to), `rename` (label
  only), `reorder`, `remove` (never a standard folder; a non-empty one
  needs `move_to`, tickets `git mv`'d there first, someone else's lease
  stops it; its `wip-limit-<folder>` and folder go) and `set_wip`; each
  re-checked by `Workflow::parse` before saving. CLI: `amd column [add|
  rename|remove|move|wip]` (`cmd_column`, each one commit), `amd move
  <ref> <column>`, and any column's name as a destination verb (`amd
  review 12`, with `--force`; any todo-kind column goes through
  `enter_todo`, `TodoOptions.to`). `amd board` heads a renamed column
  with its name; `amd log` skips dotfiles (`.gitkeep`). The MCP schemas
  take any column name. amdui: `amd_ui_model::Column` mirrors amd-lib's
  (folder + `Kind`, the standard four as constants; on the wire a
  standard column is its folder and another `"folder:kind"`), and
  `Board.workflow` (`ColumnInfo {column, label, wip}`) is what the page
  draws (`BoardView`'s `For`, labels, `is_collapsible` for backlog and
  done kinds) and what the drag rules take (`epic::can_move(&workflow,
  …)`, `sprint_rule`/`draft_rule` by kind, `wip::admit` with a column's
  own limit). `boards::ui_workflow` builds it from the tickets board; a
  declared repository's ticket in a folder that workflow lacks shows in
  the first column of its kind. Admin → Columns (`ColumnsAdmin`; commands
  `add_column`, `rename_column`, `move_column`, `remove_column`,
  `set_column_wip`, all through `change()`). **Flow, landing and reset**
  (ticket 225): `flow = next|any` (`workflow::Flow`; `next`, the
  default, is the next column plus `moves = triage>todo, …`, extra
  one-way moves) and `new-tickets = <backlog-kind folder>` in the tickets
  `.settings`, applied by `Workflow::configure` in `Board::read_workflow`
  (a bad value falls back, reported by `workflow_problem`).
  `Workflow::allows` replaces `next_to` in `Board::move_task`; every other
  rule (drafts, started sprints, done waits, WIP, leases, the branch on a
  todo kind) still applies. New tickets land in `workflow.landing()`
  (`NewTicket.land_in`, `Draft.column`); one given an epic lands in the
  standard backlog (`create_ticket`). `columns::{set_flow, allow_move,
  set_landing, reset}`: `reset` moves each ticket in an added column to
  the standard column of its kind (`git mv`, lease checked first), then
  clears `columns`, `flow`, `moves`, `new-tickets` and every
  `wip-limit-<folder>`; `remove` drops moves naming the column and a
  landing there. CLI `amd column flow|allow|disallow|landing|reset`.
  amdui: `ui::Workflow {any, moves, landing}` and `allows`, which
  `epic::can_move` uses; each `RepoInfo` carries its own board's
  workflow and drags follow the ticket's own (`Board::workflow_of`), as
  amd-lib does, since a declared repository's board reads its own
  `.settings`. Admin → Columns: Moves select, extra moves (× and Allow),
  New tickets land in, Reset to standard (with a confirm). The MCP
  `board` says the flow and landing column.
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
- **Tags are UPPERCASE and can come from a project list** (ticket 122).
  Every writer stores them uppercase (`task::normalize_tag(s)`: `Draft`,
  `apply_edit`, `set_text`), and every match ignores case
  (`Task::has_tag`, `card_visible`), so older lowercase tickets still
  match. The project's list is `tags` in the tickets `.settings`
  (`board_settings::{tags, add_tag, remove_tag}`), with `prevent-arbitrary-tags` (formerly `tags-only`, still read and renamed on the next write; Admin's "Prevent arbitrary tags", **on unless set to
off**, ticket 141) to
  refuse unlisted tags (`check_tags`: a warning, or an error when on).
  **Every list starts with the built-ins** (DRAFT, TRIAGE; ticket 167):
  a project that has listed nothing else allows only those while the
  setting is on (it used to allow any tag with no list), the form and
  `TagPicker` offer the list, and `amd tags` prints a `built in:` line
  rather than counting them as unlisted. Ticket 167 also re-cased and
  listed the tags in use on agile-md and fantastic-tools once, by hand
  (lite-actions' tickets weren't committed, so it was left).
  `model::remove_unused_tag` refuses a tag any ticket on the project's
  boards carries (`model::tag_counts`, archive excluded), so removing one
  never changes a ticket. CLI: `amd tags [add|remove|only on|off]`, and
  `amd new` warns or refuses. amdui: Admin's Tags section (add form,
  remove-unused dropdown, the switch), the editor's `TagPicker`, and
  `save_ticket` checks the list; `Board.tags`/`tags_only` carry it to the
  page.
- **Drafts** (ticket 114): a ticket tagged `DRAFT` (`task::DRAFT_TAG`,
  `Task::is_draft`/`set_draft`) can't leave the backlog (`Board::move_task`)
  or go into a sprint (`Board::set_epic`); an epic, edits and abandoning are
  fine. DRAFT is one of `task::RESERVED_TAGS`: `check_tags` always allows
  it, `add_tag` and `remove_unused_tag` refuse it, and amdui's `TagUse`
  carries it as `reserved`. CLI: `amd new --draft` (the form asks),
  `amd draft`/`amd ready <ref>`. amdui: a dashed DRAFT badge on the card,
  a Draft banner with Mark ready in the editor, "Save as draft" in New
  ticket, and `amd_ui_model::epic::draft_rule` refuses the drag. Named
  commands win over epic names, so an epic called `draft` or `ready` is
  reached through `amd group`/`amd set epic` instead.
  **TRIAGE** (`task::TRIAGE_TAG`, ticket 158) is the other reserved tag:
  what an imported GitHub issue arrives with. It holds nothing back (any
  move is fine); taking it off is triaging. Check for DRAFT itself, not
  `is_reserved_tag`, when deciding whether a ticket is a draft.
  **A TRIAGE ticket is always high and heads My work** (ticket 248):
  `Task::priority()` answers high for one whatever its file says;
  `ensure_priority` (so `Draft::write`, `set_text`, `apply_edit`) stores
  high; `set_priority` and `auto_priority::set_by_hand` refuse anything
  lower ("take TRIAGE off first"), and `write_priority` keeps it high.
  `my_work::triage` gathers every open TRIAGE ticket (not done-kind, not
  archived projects) from every project, hidden ones too, unassigned or
  yours; `my_work` puts them first (`WorkItem.triage`), and `amd mine` and
  amdui's `home` call `show_projects_of` on them so a hidden project comes
  back. The landing page marks them TRIAGE; the editor says why the
  priority is high.
- **Tag filtering**: `amd tag <tag>` lists every ticket carrying `<tag>`,
  across every column, via `Board::tasks_tagged()`/`Task::has_tag()`. Every
  listing form also takes `--tag <tag>` to narrow it down: `amd board`,
  a bare destination verb (`amd todo`, `amd backlog`, …)
  and an epic by name (`amd checkout --tag urgent`) all thread it through to
  `render::columns()`/`render::named()`, which filter before building the
  tree. Moving a ticket (`<ref>` given) ignores `--tag` — it only applies to
  the no-ref, listing form. The `Verb` external-subcommand path (epic names)
  parses `--tag`/`-t` out of its raw args by hand, since clap hands that
  whole tail over as an opaque `Vec<String>`.
  **`--tag` is repeatable and `-TAG` leaves a tag out** (ticket 123,
  `Task::matches_tags`, whole tags, case ignored): several wanted tags need
  just one of them, every `-TAG` still rules tickets out, so `--tag FUTURE
  --tag -BUSINESS` is "tagged FUTURE, not BUSINESS". clap needs
  `allow_hyphen_values` on the flag for the dash. The MCP `board` tool's
  `tag` takes the same.
- **Board search** (ticket 123, `TicketSummary::matches` in
  `amd-ui-model`): plain `word` / `-word` search the text (id, title, file
  name, body, assignee, priority, epic; ignoring case) and **never the
  tags**, so `triage` doesn't find the tag TRIAGE. `#TAG` / `-#TAG` search
  the tags: any tag *starting with* what's typed, case ignored; several
  `#TAG` terms need just one, each `-#TAG` rules tickets out, plain words
  must all appear. Tags are checked first, so the text of a ticket they've
  ruled out is never lowercased. The search box (`SearchBox` in
  `amd-ui-web/src/app.rs`) offers the tags starting with a `#T` / `-#T` being
  typed (`tag_being_typed`, `complete_tag`), from `Board.tags`. The CLI's
  `--tag` is whole tags, not prefixes.
- **Leases** (`docs/adr/0007-concurrency-and-locking.md`, currently only
  surviving at `~/develop/github/agile-md/to-sort/old-amd-docs/adr/` —
  `docs/adr/` was never restored into this repo after the git-history reset):
  `locked-by`/`locked-until` frontmatter, stored as an **absolute expiry**
  rather than a duration so every check (`Task::write_checked`,
  `Board::move_task`/`archive`/`set_epic`) is a self-contained
  `now > locked_until` comparison against data already in the file — no
  `Board` handle needed except at the one place a duration is actually
  chosen (`Task::take_lease`, called from `amd lock` and from
  `transition::move_to` on entering `doing`, which both front ends use, both of
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
  **A ticket is locked exactly while it's in doing** (ticket 156; it used
  to be todo or doing): `move_to` takes the lease on entering `doing`, and
  calls `Task::release_if_mine` on leaving it in any direction or on
  landing in `done` or `backlog` (ticket 092), inside the same
  `with_auto_commit` closure, so the move and the release are one commit.
  `Board::archive` does the same before archiving (best-effort; the archive
  is untracked). A lock taken by hand (`amd lock`) on a backlog or todo
  ticket stays until it leaves doing, done or the backlog takes it.
  `release_if_mine` only ever clears the current identity's own lease,
  lapsed or not; someone else's, even lapsed, stays for `unlock --force`.
  **A lapsed lease is said as such** (ticket 110): still recorded, it
  holds nothing, but amdui's `TicketSummary`/`TicketDocument` carry it as
  `lapsed` beside `lock` (which stays "in force" only, so no write check
  changed). The editor says "Your lock lapsed at …" / "Locked by X until
  … (lapsed)", the card badge is dashed, and `amd show` prints the same on
  stderr (`lock_note`), leaving stdout exactly the file.
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
  duration, default 8), `AMD_WIP_LIMIT` (doing's WIP limit — unset or
  unparsable defaults to **2**, not unlimited; `0` turns it off; todo is
  never limited, so `AMD_WIP_TODO` is ignored since ticket 083), `AMD_COMMIT` (auto-commit-and-push,
  unset = **on**; `0`/`off`/`false` turns it off), `AMD_GIT_BASE` (where
  `amd join` clones from, default `git@github.com:`), `AMD_SYNC` (catch up
  with the remote, default on, ticket 102), `AMD_TICKET_NUMBERING`
  (`local`/`cloud`). **`AMD_TYPES` is dead** —
  it was in `--help` long after nothing read it. **`AMD_NO_REGISTER` is also
  gone** (2026-09-26) — registration is a deliberate act (`amd register`)
  again, not something running in the background to opt out of.
  `AMD_BRANCH_TYPES` and `EDITOR` also fall back to the settings file
  (`branch-types`/`editor`) when unset — the env var wins when both are set
  (`settings::resolved()`). `AMD_LEASE_HOURS`/`AMD_WIP_LIMIT`/
  `AMD_COMMIT` follow the same precedence against the *board-local*
  `.settings`'s `lease-hours`/`wip-limit`/`commit` keys instead — team
  decisions, not personal ones. `wip-limit` (2026-09-27) replaced an
  earlier `wip-doing` key entirely.
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
    `amd new`) also writes the `.next-id` counter, so a new ticket
    genuinely touches two files in one operation, and nobody wrote that
    call site expecting to enumerate the counter too.
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
  - **`dirty_paths` reads `git status -z`** (ticket 204): without it git
    quotes a path with a space, so an epic called "Launch plan" gave
    `"tasks/backlog/Launch plan/_group.md"`, quotes included, and the
    commit's pathspec matched nothing: every change to such a group failed
    to commit. A rename's original path is the next NUL field.
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
    moves, not one per ticket — a single logical action), and declaring or
    undeclaring a repository (`amd register`, `register --remove`,
    `deregister`). Not in scope: this machine's registry alone (a
    `deregister` of a repository that's only registered here),
    `{project}-documents` (a different repository; ADR 0007 moves documents
    to direct pushes too, but amd doesn't write to them yet).
  - **Sync with the remote** (ticket 102, `crates/amd-lib/src/sync.rs`).
    Before it, nothing fetched or pulled, and a rejected push only warned:
    the multi-user suite (076) proved duplicate ids, a lease claimed twice
    and invisible teammates' tickets. Two halves:
    - **`catch_up(board, Purpose)`**, before a command: the CLI calls it
      once in `run()` (`sync_purpose` classifies each command: a ticket
      ref = Write, a listing = Read); amdui calls it in `change()` (Write)
      and every 30s from the page (`sync_board`, Read). It fetches (a Read
      skips that if amd's last fetch was under 10s ago), then: only behind →
      `merge --ff-only`; only ahead → push the leftovers; diverged → a
      Write refuses with the fix, a Read warns. No remote/upstream, or
      `AMD_SYNC=0`/`sync = off`, does nothing; no network only warns.
    - **amd's fetch never writes `FETCH_HEAD`** (ticket 118,
      `git::fetch`): `--no-write-fetch-head`, with `.git/amd-fetched`
      touched instead as the "fetched recently" timestamp
      (`fetched_within`). `FETCH_HEAD` belongs to `git pull`, which writes
      it and reads it back; amdui's 30s fetch landing mid-pull made the
      pull fail with "Cannot fast-forward to multiple branches" (142 of 200
      pulls in a race loop, 0 after). This is why amd needs **git 2.29+**.
    - **`after_rejected_push`**, from `transition::commit_since` (now
      commit-locally, then push — and `cmd_new` goes through it too): a
      push rejected because the remote moved is replayed with `rebase @{u}`
      when that's clean **and** `Board::duplicate_ids()` is empty; otherwise
      the written files are copied to `$XDG_STATE_HOME/agile-md/recovered/`,
      the commit is dropped (`reset --keep`), the clone fast-forwarded, and
      `ConcurrentChange` returned — an `Err`, so a non-zero exit. Any other
      push failure keeps the old warn-and-keep-local behaviour; the next
      catch-up pushes it. **Note:** two clones bumping `next-id` to the
      *same* value rebase cleanly (identical change), which is exactly why
      the duplicate-id check exists.
    - **Two warnings for a board being trashed** (ticket 145): a
      fast-forward that removes `MASS_REMOVAL` (5) tickets or more still
      happens (it's recoverable), with a warning naming the count, ids,
      commits and authors and pointing at `amd recover`
      (`removed_tickets`). And when commits this clone had from origin
      are no longer in origin's history (`rewritten`: an earlier position
      of the upstream, from its reflog, is in HEAD but not in `@{u}`), a
      force push rewrote it: a Write refuses and a Read warns, naming it as
      such, and nothing is pushed back or reset; the message points at
      `git push --force-with-lease` for a person to decide. Checked only
      when the clone is ahead, so an ordinary divergence never trips it.
    - `tests/test.sh` forces the race deterministically with a one-shot
      `pre-push` hook in clone B that makes clone A push first (`race()`,
      clearing git's per-repo env with `git rev-parse --local-env-vars`).
  - **Ticket numbering** (`crates/amd-lib/src/numbering.rs`): `local` (the
    counter + scan, unchanged, the default) or `cloud` — a stub for a future
    paid plan where a service hands out numbers; it refuses with "needs an
    agile-md plan". `ticket-numbering` lives in the tickets `.settings` like every setting;
    `AMD_TICKET_NUMBERING` overrides. `Board::next_id`/`record_id` dispatch
    through it, so no caller changed.
  - **`amd log`** (`cmd_log`) renders the board's transition history —
    `date time action id title sha` — from `git log --name-status -M`,
    reading paths and rename status only, never the commit message, so it
    works retroactively over history from before this feature existed too.

## Ticket fields (`crates/amd-lib/src/branch.rs`, `templates/ticket.md.jinja`)

- **A repository and a branch type go together** (ticket 066,
  `crates/amd-lib/src/pairing.rs`), on a dedicated tickets board only
  (`Board::is_tickets_board`, moved from `transition`: the repository is
  named `*-tickets`; a `--local-only` board in its code repo is exempt).
  `branch::NO_BRANCH` (`none`) is a branch type meaning "no branch, on
  purpose" (`makes_branch` false, no `branch-name`, `amd todo` silent);
  the picker's empty choice is `branch::NONE`, now "(not chosen)" (old
  "(none)" still normalises to empty). `pairing::mismatch(repository,
  branch_type)`: a repository with no branch type, or a real branch type
  with no repository. Not refused: the ticket becomes a **draft**.
  `pairing::check` answers `Some(why)` in a backlog-kind column and errors
  past it. Applied in `Draft::write` (every creation path; `Created.drafted`
  says why, the CLI prints it, `create_ticket` adds a warning),
  `transition::save_text` (DRAFT put back even if the editor took it off),
  `edit_ticket` and the MCP `edit_ticket` (`check_pairing` before,
  `settle_pairing` after), `amd set <ref> repository|branch-type`
  (`Task::set_branch_type`, new), `amd import tickets` (`ImportedTicket.
  mismatched`, a note), and `new_repo::record_on_ticket` (now returns a
  note: past the backlog it can't refuse a repository already made). `amd
  ready` refuses (`pairing::ready`). Fixing the fields never takes DRAFT off.
  `amd check` lists mismatched tickets not done (`pairing::mismatched`),
  read only. The MCP `create_ticket`/`edit_ticket` take `branch_type`.
  amdui: `RepoInfo.tickets_only`, `Board::tickets_only(repo)`,
  `amd_ui_model::ticket::{NO_BRANCH, pairing_mismatch}` (amd-ui's tests
  check it words things as amd-lib does); the editor's Branch type has "not
  chosen" and "none (no branch)", a note under it, and Mark ready disabled
  with the reason.

- **Due dates and recurring tickets** (ticket 143,
  `crates/amd-lib/src/recur.rs`): `due: "YYYY-MM-DD"` on any ticket
  (`Task::due`/`set_due`, empty removes the key) and `recur: "<rule>"`
  (`Task::recur`/`set_recur`, stored in `Every`'s plain form: daily,
  weekly, fortnightly, monthly, quarterly, yearly, `every N
  days|weeks|months|years`; months clamp to the month's last day).
  `transition::move_to`, on a move into a done kind from another, calls
  `recur::make_next` inside its `with_auto_commit`, so the next ticket
  and the counter are in the move's commit: a new ticket (Draft, landing
  column) with title, body through `clone_body`, tags, points, priority,
  repository, branch type, assignee and the rule, `recurs-from` the old
  id, and `due` from `next_due` (one interval after the old due, else
  today, stepped past today). No daemon: overdue (`recur::overdue`: past
  due, not done-kind) is only shown, on the card (red "overdue"), in `amd
  show` (stderr) and `amd board` (`overdue:DATE`, `repeats:RULE`). Both
  are checked on every write path (`set_text` too). CLI `amd new --due
  --recur`, `amd set <ref> due|recur`; amdui: the editor's **Due** (date)
  and **Repeats** (select, a hand-written rule kept), `Fields.due/recur`,
  `TicketSummary.due/recur`, the card's due badge and ↻. Not in the MCP
  tools yet.
- **Due dates on the board** (ticket 253): `transition::straight_to_todo`
  moves a loose-backlog, non-draft ticket due within `due-to-todo-days`
  (default 3, 0 never; `board_settings::due_to_todo_days`) to the first
  todo-kind column, stamping `todo-since`, and says so; a refused move is
  only a note. Called after `amd new --due`, `amd set due`, `save_text`
  when the due date changed, and `recur::make_next` (in `move_to`).
  `Board::refuse_due_in_epic` (from `Board::file`, so every `set_epic`, and
  from `amd set due` and `save_text`) refuses a due-dated ticket in an
  epic; sprints may hold them, and `amd <sprint>` warns on stderr.
  `backlog-always-shown` (`board_settings`) stops amdui folding the
  standard backlog. CLI `amd due-to-todo [DAYS]`, `amd backlog-always-shown
  [on|off]`. amdui: `Board.due_to_todo_days`/`backlog_always_shown`,
  Admin's "Due dates and the backlog" (`set_due_to_todo_days`,
  `set_backlog_always_shown`), `amd_ui_model::ticket::due_first` orders
  backlog-kind columns, `epic::due_rule` refuses the drag onto an epic, the
  sprint column warns, and a 13-point card has a red border.
- **Duplicating a ticket** (ticket 198, `amd duplicate <ref>`, the editor's
  Duplicate button, the MCP `clone_ticket`; `transition::clone_ticket`). `amd
  clone` was already taken (it clones a declared repository). The copy goes
  to the same board's backlog with the title, body, tags, branch type,
  repository, parent, priority and points; not the assignee, lease, epic or
  `created*`; the body loses its `## Children` and its ticked items are
  unticked. It is linked by a typed pair, not `related`: `clone-of: "NNN"`
  on the copy (only written when set) and `clones: [..]` on the original
  (`Task::add_clone`, written by `Draft::write`). `import` and `renumber`
  remap both keys, `renumber` using each end as proof of the other. An
  original someone else holds can't take the backlink: the copy is still
  made and `Created.unlinked` becomes a warning. Same board only, since a
  bare id can't name a ticket on another.

- **Save and New** (ticket 227): the editor's button saves the ticket
  (when it has edits) and opens `Overlay::NewTicket(Some(LinkBack))`,
  whose "Link to" choice (`LinkKind`: requires, the default, related, or
  none) goes to the `create_ticket` command as `required_by`/`related_to`.
  `TicketFields.required_by` links each named ticket as requiring the new
  one in the `new:` commit; the new ticket goes on the linked ticket's
  board.

- **Typed links** (ticket 207, `crates/amd-lib/src/links.rs`, `amd link
  <ref> <ref>... [--type T] [--remove] [--one-way]`, the MCP `link_tickets`,
  the editor's **Add link**). `LinkType` has nine kinds, each with an
  inverse written on the other ticket: relates-to, requires/required-by,
  parent-of/child-of, clone-of/cloned-by,
  supersedes/superseded-by (the last of each pair exists only so the other
  end has something to say). The first version's names are still read
  (`LinkType::parse`): depends-on and blocked-by are requires,
  depended-on-by and blocks are required-by, and an old entry is written
  back under its new name. **Four kinds keep their older fields** so
  there is one record of each fact: relates-to is `related`, child-of the
  child's `parent` with parent-of the parent's `## Children` wikilink,
  clone-of `clone-of` with cloned-by `clones`. The rest live in a `links`
  flow list of `kind:id` entries (`links: [requires:012,supersedes:003]`,
  `Task::links`/`add_link`/`remove_link`; an entry it can't read is skipped).
  `links::apply` writes both ends, each hash-checked, after a `preflight`
  that refuses a second parent or a second original before touching a file;
  `transition::link_tickets` wraps it in one auto-commit (`link:`/`unlink:
  001 -> 002`). Same board only. `import` and `renumber` remap `links`
  (renumber: each end vouches for the other as the inverse kind). amdui:
  `TicketSummary.links` carries the entries; `amd_ui_model::LINK_KINDS`
  mirrors `LinkType::ALL` (amd-ui's tests check they agree, as the model
  can't depend on amd-lib); `link_candidates` is the search (a number finds
  that ticket first, words match title or file name); the Relationships
  panel shows a group per kind with a × on each, and both Add link and ×
  wait while the editor has unsaved edits, since they rewrite the file.
  The command is `link_ticket`.
  **Every ticket linked to one, either way** (ticket 121):
  `links::linked(board, task)` reads the ticket's own links (`links::all`
  plus its `## Children`) and every other ticket's naming it, turned round
  with `LinkType::inverse`, so a one-sided link is found from both ends;
  `Linked {kind, task, recorded_here}`, each ticket and kind once, in the
  picker's order. CLI `amd links <ref> [--json]` (alias `amd related`);
  the MCP `ticket` tool adds a "Linked" list and `data.linked`; the
  editor's Relationships panel also shows a typed link recorded only on
  the other ticket, as its opposite (`amd_ui_model::link_inverse`, which
  amd-ui's tests check against `LinkType::inverse`).
  **Done waits** (ticket 213): `Board::move_task` refuses a move to done
  while `links::waiting_on` finds tickets on the board, not done, that the
  ticket requires (its own `requires`, or another's `required-by` naming
  it) or that are its children (their `parent`, or its `## Children`
  list), returning `links::NotReady` naming each with why and where.
  **No override**, `force` included (that's only the WIP limit): the way
  past it is to change the link (`links::retype`, `transition::retype_link`
  → `relink: 001 -> 002`; `amd link A B --replace OLD --type NEW`, the MCP
  `link_tickets` `replace`, amdui's per-link type picker and its
  `retype_link` command) or remove it. Abandoned or missing tickets hold
  nothing back. The page mirrors the rule (`amd_ui_model::waiting`): a
  "waits on N" badge on the card, and `verdict` refuses a drag onto done
  with the reason, which columns now show as epics did.
  **Requires and sprints** (ticket 214, `crates/amd-lib/src/requires.rs`):
  there is no separate depends-on field; requires *is* the dependency,
  called requires everywhere. `requires::required(board, task)` is the
  one reading (own `requires` + others' `required-by`), used by
  `waiting_on`, the sprint check and the tree. `sprint_gaps(board, name)`
  lists each ticket in the sprint (`tasks_in_group`) that requires one
  neither in it nor done; `requires::start_sprint` refuses with
  `SprintNotReady` (`s1 can't start: 003 requires 001 (backlog)…`), and
  `amd group start` and amdui's `start_sprint` both call it instead of
  `Group::start`. Nothing is enforced outside sprints. Setting them:
  `amd new --requires <ref>` (repeatable; resolved before writing, linked
  in the `new:` commit), `amd set <ref> requires 3,7` (exactly that list,
  both ends, empty clears: `transition::set_requires`),
  `TicketFields.requires` (create only; the MCP `create_ticket`
  `requires`). The page mirrors the check (`amd_ui_model::waiting::
  sprint_gaps`): the sprint column disables Start sprint and lists the
  gaps.
  **The tree** (`requires::family(board, around)`, `amd tree [<ref>]`):
  one board's families, tickets joined by requires or related either
  way, as serialisable `TreeNode`s (id, title, file, column, epic, place,
  done, edge Root/Requires/Related, unmet, stub, seen, current). `Facts`
  reads every ticket's links once. Roots: tickets nothing in the family
  requires, those that require something first (so a ticket only related
  to one hangs under it); a pure cycle starts at its lowest id. Each
  ticket drawn once (`seen` back-references carry no marks); related
  drawn once, under whichever end comes first. Tickets in a started
  sprint (`!accepts_changes()`) are stubs, not expanded and not walked
  through; asking for one is refused. Parent/child aren't drawn. A lone
  ticket has no family (empty). Done tickets are left out unless
  `include_done` (ticket 217, `Facts::hide_done`: their links dropped, so
  a family held together only by a done ticket comes apart), except the
  ticket asked about; `amd tree --done`, and the panel's "Include done"
  checkbox (off each time it opens). An empty answer only because of
  done tickets says so. render.rs `tree`: `├──── ` solid,
  `├ ─ ─ ` dashed (spaced so related stands out), both six wide; colour
  and links only on a terminal.
  **amdui's tree:** the `ticket_family` command returns `family`'s nodes;
  `amd_ui_model::tree::TreeNode` mirrors them (amd-ui's tests check the
  JSON agrees). A card with a family (`waiting::has_family`: requires,
  required-by or related, either end), not in a started sprint, gets a
  tree button opening `Overlay::Tree { place, file }` (`tree.rs`'s
  `TreePanel`): nested rows, a solid rule for requires and a dashed one
  drawn as a repeating gradient (4 px dash, 6 px gap) for related, the
  ticket it was opened from highlighted, waits-on marked, stubs greyed.
  A click on a node goes to its card with `Ctx::show_card` (the board
  shown, `focus`/`focus_scroll`, its epic or sprint column opened, a
  folded column unfolded), which `open_ticket` now uses before opening
  the editor. Each sprint gap has "Show the tree".
  **Auto priority** (ticket 215, `crates/amd-lib/src/auto_priority.rs`):
  `auto-priority` (**on unless off** since ticket 252; it was off unless
  on) and `priority-override` (on unless off)
  in the tickets `.settings`, read per board. When on, a ticket not done
  is at least as urgent as the most urgent not-done ticket that requires
  it, down a chain (repeated to a fixpoint; three steps, so cycles end).
  Written into `priority`, a raise recorded as `priority-from` (the
  requirer) and `priority-own` (its own before), `Task::write_priority`
  writing all of them in one checked write. `reconcile(board)` runs in
  `transition::settle_priorities`, called by `with_auto_commit` after the
  change and before the commit (and by `create_ticket` and `cmd_new`), so
  every change carries its priority moves; a failure is only a note.
  What it changed queues for `take_notes` (the CLI prints them after each
  command). Off puts raised tickets back; pins stay. By hand
  (`set_by_hand`: `amd set <ref> priority`, the MCP `edit_ticket`,
  `transition::edit_ticket`, `save_text` when the saved priority differs):
  the value becomes the ticket's own, raises cleared, and with override on
  `priority-pinned: "yes"`, which reconcile leaves alone; `auto` unpins.
  With override off a hand value is only a floor. A ticket someone else
  has leased is left alone with a note. CLI: `amd auto-priority [on|off]`
  (bare lists the raised), `amd priority-override [on|off]`, `amd show`
  prints `auto_priority::note` on stderr. amdui: Admin's Auto priority
  section (`set_auto_priority`, `set_priority_override`), the editor's
  `auto` option (the page's `ensure_priority` accepts it) and note
  (`TicketDocument.priority_note`), and `↑ high` on a raised card
  (`TicketSummary.priority_from`). **Due dates** (ticket 252): `due_rank(due, created, today)` is
  `time_rank` over the window from `created` to `due` (a quarter: medium,
  half: high, three quarters: top, so top once overdue; no `created`: top
  only once overdue); it's a ticket's starting point before the requires
  chain, so it passes down, recorded as `priority-from: "due"`
  (`auto_priority::DUE`; `note` and the card say "due DATE"). No setting.
  `reconcile` is now `plan` + apply, `reconcile_at(board, today)` takes the
  clock, and `pending(board, today)` says whether it would write. Nothing
  changes on disk as days pass, so amdui's `sync_board` calls
  `raise_for_due_dates` (hourly a board, `AppState.due_checked`), which
  commits `priority: due dates` through `with_auto_commit` when `pending`.
  **`top`**
  is a fourth priority (`task::TOP`, `PRIORITIES` has four, the model's
  too): every TRIAGE ticket is top (was high), and anyone can choose it.
  **Time limits** (`todo-max-days`, `doing-max-days`,
  `board_settings::TimeLimits`, 0 none): `transition::move_to` stamps
  `todo-since` (entering todo from the backlog) and `doing-since`
  (entering doing; todo's kept), both cleared in the backlog
  (`stamp_column_dates`, `Task::set_column_dates`); `time_rank(used,
  limit)` is medium from a quarter, high from half, top from three
  quarters; time in todo counts up to `doing-since`, so a raise survives
  being picked up (`time_used`). Recorded as `priority-from: "todo"` /
  `"doing"`. CLI `amd auto-priority limits [TODO DOING]`; Admin and
  `set_time_limits`; `Board.time_limits`. **Automatic sort** (ticket 230):
  `SortOrder::Automatic` ("Automatic: what's needed first") is a
  dependency order, `SortOrder::sort` (a pairwise `compare` can't express
  it): the column pre-sorted by priority then age, then each ticket placed
  only after the tickets in the list it requires on its own board
  (`waiting::requires`), a cycle broken at the first left. `Ctx::shown`
  calls `sort`; `Ctx::default_sort` is Automatic when the board's
  `auto_priority` is on, and the sort follows it until one is picked
  (`ctx.sort_chosen`); Reset filters goes back to it.
  **How linked a card is** (ticket 216, `waiting::relationship_count`):
  the number of other tickets it's linked to in any way (any `links`
  kind, related, parent and children, clones), from either end, each
  once. A card with a tree shows it beside the tree icon; one linked
  otherwise (a parent, a clone, a started sprint's ticket) shows a link
  icon and the count; one with none shows a small hollow blue dot.

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
- **`priority`** is `low`, `medium` or `high` (`task::PRIORITIES`), `low`
  by default — the built-in template writes it, and `Draft::write` fills it
  in for a custom template that doesn't (`task::ensure_priority`), and
  refuses an invalid one. `Task::priority()` reads a missing one as `low`;
  `amd set <ref> priority <value>` sets it (ticket 083).
- `assignee` says who's doing it, empty by default: tickets are created now and
  assigned later. It was briefly called `owner`; `Task::assignee` still reads
  that key. **Who created it is stored; who last edited it isn't** (ticket
  155, reversing an earlier "no second person field" rule): `created-by:
  "Name <email>"` is written once at creation (`TaskContext.created_by`,
  the built-in template; `Draft::write` fills it for a custom template via
  `authorship::ensure_created_by`) and never changed. "Last edited by" is
  read from git (`authorship::last_edited`: the latest commit changing the
  file, a pure `R100` move not counting), because a stored copy would go
  stale on every edit made outside amd. Names come from the users list by
  email. A ticket from before 155 is credited from the first commit adding
  its file name and gets the field on its next save (`transition::save_text`).
  `amd show` prints `authorship::summary` on stderr; amdui's editor shows
  it under the lock bar (`TicketDocument.authorship`). **Picked from the project's `users`** (ticket
  089): `amd new`'s form and a bare `amd assign <ref>` in a terminal open
  `form::assignee` (type-to-filter: nobody, `@me`, the users, someone
  else); amdui's editor has `AssigneePicker`, fed by `Board.users`/`me`.
  The *name* is stored. A name off the list is allowed with a warning
  (`board_settings::unknown_assignee`; never for a board with no list).
  Without a terminal, bare `amd assign` still clears; `--clear` says so.
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
  The branch goes in the clone of the ticket's `repository` (ticket 107).
  **A dedicated tickets repository never gets one** (ticket 053,
  `transition::enter_todo`): on a board whose repository is named
  `*-tickets`, a ticket that wants a branch but names no repository is
  refused before the move, naming `amd set <ref> repository` and
  `--no-branch`. A board inside a code repository (`init --local-only`
  there) is that repository and branches as before.

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
  printout only; the interactive board is `amdui`.)
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
  `branch_type`, `branch_name`, `owner`, `parent`, `parent_link`, `related`, `clone_of`, `tags`, `created`, `timestamp`, `column`,
  `author`, `email`, `created_by`, `board`, `template`, `extra`). Adding a field is additive;
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
  actually cloned on *this* machine, and where. *Which* names are worth
  asking about always comes from the current project's own declared list,
  never from scanning this file's entries wholesale. **Lookups are scoped
  by project directory** (ticket 095): the same project name can live in
  two folders, so `Registry::find_near(name, project_dir)`/`clone_of(from,
  name)` take the clone in the current project's folder, else the only
  clone of that name anywhere, else nothing (ambiguous → shown as not
  cloned, never a guess). `find(name)` is only for `deregister`. A path
  that no longer exists is skipped on load and so dropped on the next
  save. Registering (`init`, `join`, `register`) a clone of a remote
  that's already registered elsewhere **warns** (`same_remote`: `origin`s
  compared via `git::normalise_remote` — case, scheme, user, port, `.git`,
  SSH host aliases resolved with `ssh -G`; no `origin` falls back to the
  first commit), never refuses. There is deliberately no
  store of tickets in either half: the markdown is the source of truth, it
  changes under you on every checkout and pull, and an index would be
  stale the moment it was written.
- **`amd register [path]`** (defaults to the repo you're in) does both
  halves at once — declares the name for the project, registers the path
  locally — and is the *only* writer now. Registration is deliberate again:
  `registry::remember()`'s run-on-every-command auto-fill is gone, along
  with `AMD_NO_REGISTER` (nothing left to opt out of).
- **Which project `amd register` declares on** (ticket 103,
  `register_board` in `main.rs`): normally the board you're in. From a
  repository with no board of its own, in a folder holding several
  projects, that's whichever `.current-project` names, so it **asks** in a
  terminal ("Register X with project P (this folder's current project)?")
  and **refuses** without one, naming the fixes. It hints `--project Y` when
  the repo's name starts with another project's name. `--project <name>`
  always says outright. `deregister` (and `register --remove`) go through
  the same check, worded "Take X off project P?" (ticket 120).
- **`amd deregister <name>`** (ticket 120, reversing the earlier
  local-only design): a repository the project **declares** comes off the
  project for everyone, exactly as `amd register --remove` (now an alias):
  undeclared in the tickets `.settings`, committed and pushed, and dropped
  from this machine's registry. The repository filter and fields list the
  declared repositories and nothing else (`ticket_repos`, ticket 195: a
  repository a ticket merely names never adds to them), so it leaves
  everyone's lists. A repository
  that's only *registered* here (a project's own tickets or documents
  repository, a scratch board) just loses its registration. Neither is an
  error. The clone is never touched. It takes `--project` like `register`.
- **`amd repo new <name> [--for <ref>]`** (ticket 228,
  `crates/amd-lib/src/new_repo.rs`): makes a repository for the project,
  or connects one that's already there, from where the tickets
  repository's `origin` is (`new_repo::forge`): on GitHub under the same
  owner through `gh` (a closure, `new_repo::gh`, so tests pass a fake),
  beside a local origin as a bare repository (what test.sh and the demo
  use), or only here (`git init`) with no origin; another forge is
  refused. `create` checks the name as typed (`validate_project_name`),
  refuses one already cloned or in the way, defaults visibility to the
  tickets repository's, clones with `clone::sibling_url`, adds a starter
  README on an empty one (pushed), and protects `main` on GitHub
  (`github::protect_main`, now in amd-lib). The caller then `register`s
  the clone (the machine's registry, kept out of `create` so tests never
  touch it), and in one commit `declare`s it and `record_on_ticket`
  (repository set, a signed, dated note via `task::with_note`, moved from
  assistant.rs). amdui: the editor's Repository select has "＋ New
  repository…" (admins only; it saves the ticket first if edited), the
  `NewRepoDialog` (`repo_defaults`, `create_repository`, which checks
  `boards::admin_access`), then reopens the ticket. Not in the MCP tools.
- **`amd clone [name | --all]`** (ticket 108, `crates/amd-lib/src/clone.rs`):
  clones a declared repository this machine hasn't got into the project
  directory, from the tickets repo's own `origin` with the last path part
  swapped for the name (`sibling_url` — keeps an SSH host alias, works
  with local bare repos), then registers it. No name lists the missing
  ones. Refuses an undeclared name, one already cloned for this project,
  and a directory in the way. amdui offers the same from the Repository
  filter: a "(not cloned — clone…)" entry opens `CloneRepoDialog`
  (`clone_repository` command); the editor's select stays disabled.
- **There are no repository aliases** (removed in ticket 097). The
  machine-wide `tickets`/`documents` short names ticket 026 added only ever
  pointed at whichever project last ran `init`/`join`, so with several
  projects on one machine they misled. Repositories go by their real
  `{project}-tickets`/`-documents` names; `amd deregister` takes the real
  name. A leftover `$XDG_CONFIG_HOME/agile-md/aliases` file is ignored.
- **`model::load_repos()` cross-references the two.** `Board::discover()`
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
(cd crates/amd-ui-web && trunk build --release)   # the board's page, before anything builds amd-ui
cargo test                          # the whole workspace
cargo build -p amd-cli              # the CLI alone: no window or web stack
bash tests/test.sh
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy -p amd-ui-web -p amd-ui-model --target wasm32-unknown-unknown -- -D warnings
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
- **A started sprint takes nothing more and gives nothing back** (ticket
  083, replacing ADR-0009's "in and out" rule — the desktop board's rules
  were adopted in amd-lib). `Board::set_epic` refuses filing a ticket into
  a started sprint and taking one out of it, in any column; its tickets
  only move on, to todo (`move_task`), and archiving straight out of one is
  refused too. Any group refuses an unsized ticket if it is a sprint.
  **Closed and backfill** (ticket 052, RFC 0005,
  `crates/amd-lib/src/sprints.rs`): `group::State::Closed`
  (`started: "closed"`); `accepts_changes()` is false for started and
  closed, `locked_word()` says which. `sprints::close` (a started sprint
  only, no all-done precondition): unfinished tickets in its backlog folder
  go back to the loose backlog through `Board::file(task, None, true)`
  (`deliberate` skips only the started-sprint rule), todo/doing ones keep
  the label, then `Group::close`; `stats::burn_of` takes the burn first.
  `file_report` writes `sprint/history/<YYYYMMDDTHHMMSSZ>-<slug>.md` and
  `.svg` (`report_text`, `report_svg`) into the project's own documents
  repository through `commit_change` (straight to main, ADR 0007, so no
  Forge). `sprints::backfill(board, name, points)`: a started sprint only;
  loose-backlog tickets (never an epic's), sized, unassigned, not drafts,
  not leased by another, by priority then `created` then id, each that
  fits under the points, filed with `deliberate`. CLI `amd group close`,
  `amd group backfill <name> --points N`; amdui `close_sprint` (files the
  report, a warning when documents aren't cloned), `backfill_sprint`, the
  running sprint's column has Backfill (points) and Close sprint (confirm);
  `Epic.closed`, and the page treats closed like started in its rules.
- **The loose backlog is the default epic.** A ticket in any epic or sprint
  moves straight on to todo (`move_task` from its folder) and keeps its
  `epic` there. Sent back from todo, `Board::home_group` puts it back in the
  group its `epic` names — unless that group is gone, is a started sprint,
  or is a sprint it no longer has points for; then the loose backlog.
- **Moves go one column at a time** (`Column::next_to`, checked first in
  `move_task`): backlog → todo → doing → done and back, never a jump.
- **`Board::rename_group`/`amd group rename <old> <new>`** moves the folder
  (`git mv` when tracked), rewrites `_group.md`'s `name`, and relabels the
  `epic` of every ticket in it, in any column — refused up front, changing
  nothing, if any of them is leased by someone else.
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
  aborts on) a ticket it can't move. `amd group empty <name>` in the CLI.
  **To todo** (ticket 256): `transition::group_to_todo` moves the group's
  tickets still in its backlog folder to the first todo-kind column
  (`move_task`, so drafts and others' leases are skipped and said),
  writing the `epic` label if it's missing and `todo-since`; no branches.
  `amd group empty <name> --to todo`. amdui: the open epic or sprint
  column's **All to backlog** (hidden for a started sprint) and **All to
  todo** (`move_epic_tickets`, one commit, skipped ones in the warning).
- **The WIP limit** (`board_settings::wip_limit`) gates entry into
  `doing` only (ticket 083: a limit on todo as well could leave work stuck,
  so `wip-todo`/`AMD_WIP_TODO` are ignored) — `Board::move_task` checks the destination column's count
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
  Doing: absent or unparsable defaults to **2** (`DEFAULT_WIP_LIMIT`; it
  was 1 until ticket 083), not unlimited, and `0` is stored *literally* by
  `set_wip_limit` rather than collapsed into clearing, specifically so
  "nobody set it" (→ 2) and "somebody set it to 0" (→ off) stay two
  different states on disk. `set_wip_limit(Column::Todo, …)` is refused.
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
  `tasks_in_group` finds any ticket still claiming membership **that isn't
  done** (ticket 204), checked across every column the same way
  `empty_group` already does — not just the physical folder — since a
  committed sprint's tickets can sit in `doing` while the backlog folder
  itself is empty. So a finished sprint or epic can go; its done tickets
  keep their `epic` label as the record. `empty` it first; `delete` does
  not imply `empty`. amdui: the open epic or sprint column has **Rename**
  (inline; the column reopens under the new name) and **Delete** (asks
  once more; disabled while `Epic.unfinished` > 0), commands `rename_epic`
  and `delete_epic`. Admin keeps its rename list too.
- **Next round of epic/sprint design, not built yet**: sprint backfill, a
  `Closed` state, and burn-up reporting into the documents repo are
  designed in `agile-md-documents`'
  `adr/rfc/0005-sprint-backfill-closing-and-burn-up-reporting.md` — including
  one question it deliberately leaves open (whether a running sprint's
  `todo` is a gate on the existing column or a nested one). Nothing below
  describes any of that; it isn't shipped.

## The desktop board (`crates/amd-ui*`, ticket 084)

- **Tauri 2 + Leptos 0.8 (CSR), built from the grimartz board** that settled
  the rules ticket 083 put into amd-lib. Three crates: `amd-ui` (the Tauri
  app, binary `amdui`), `amd-ui-web` (the page, compiled to wasm by Trunk:
  `trunk build --release` in `crates/amd-ui-web`) and `amd-ui-model` (the
  types and pure rules both sides share; it compiles to wasm, so it has no
  file, git or settings logic).
- **Every change goes through amd-lib.** `amd-ui/src/lib.rs` holds the
  commands; each opens the project as `amd` does (`boards::Project::open`,
  over `model::load_repos()`) and calls amd-lib: `transition::move_to`
  (lease + commit), `Board::set_epic`/`rename_group`/`create_group`/
  `archive`, `Group::start`, `Task::take_lease`/`release_lease`/
  `break_lease`, `transition::create_ticket`/`save_text`, `board_settings`
  for the WIP limit, lease hours and `commit`, and `Templates` for the
  template editor (`with_source` + `example_context` render a preview).
  Changes are serialised by one mutex. Don't add ticket logic to `amd-ui`;
  add it to amd-lib and call it.
- **The editor saves the whole text** (`transition::save_text` →
  `Task::set_text`): hash-checked against the `version` the editor was
  opened with (`ContentHash` as 16 hex digits, since a page can't hold a
  `u64`), priority/points validated, the lease fields always kept as on
  disk. The page derives `branch-name` from the title with
  `amd_ui_model::slugify`, which must match `agile_md::task::slugify` —
  `amd-ui`'s tests check they agree.
- **The Backlogs column lists compact boxes** (ticket 109): each epic or
  sprint shows only its name and an amount — an epic's ticket count, a
  sprint's total points — and stays a drop target. A click opens it as a
  column of its own beside Backlogs (`OpenEpicColumn`, `Ctx.open_epic`,
  one at a time, × or a second click closes it), holding its cards, the
  sprint's days/state/points line and its Start sprint button.
- **Drags map onto amd-lib's moves** (`move_in`): onto an epic or sprint is
  `set_epic` (from todo, `move_task` back to the backlog first); out of one
  to the loose backlog is `set_epic(None)`; anything else is `move_to`. The
  page previews the same rules (`amd_ui_model::epic::{can_move,
  sprint_rule}`, `wip::WipLimits::admit`) so a refused drop is shown while
  dragging; amd-lib is still the authority. A drop on a full doing asks,
  then moves with `force`.
- **The page is embedded at compile time** (`tauri.conf.json`'s
  `frontendDist` is `../amd-ui-web/dist`), so `trunk build` must run before
  anything compiles `amd-ui` — CI does it first. The `custom-protocol`
  feature is on by default so a plain `cargo build` makes a working board;
  `cargo tauri dev -- --no-default-features` in `crates/amd-ui` serves the
  page live from `trunk serve` instead.
- **It follows the files** (ticket 091). The page asks `board_fingerprint`
  every second. Like every command it opens the project with
  `Project::open`, which uses `model::find_repos` — the boards only, **no
  ticket read** (ticket 184: it used `load_repos`, which read every ticket
  into cards and dropped them, 941 file opens a second on agile-md); use
  `load_repos` only where the cards are wanted. Then: `model::fingerprint` hashes each board file's relative
  path, size and mtime (columns, epic and sprint folders, `.settings`; not
  `archive/`) across every loaded repo. When the fingerprint changes, the
  page calls `ctx.refresh()`: a reload, plus a bump of `ctx.disk_tick`. So
  `amd` in a terminal, an editor, a pull or a checkout all show up within
  about a second, with no watcher thread or dependency. A change seen
  mid-drag waits for the drop. amdui's own writes cause one harmless extra
  reload. An open editor checks its own ticket on each tick
  (`ticket_version`): if it has no unsaved edits it reopens from disk;
  with unsaved edits it shows a "changed on disk" banner (Reload / Keep
  editing); moved or removed gets its own banner. The save's hash check
  still refuses a stale write. The header's ↻, F5 and Ctrl+R all call
  `refresh()`. F5/Ctrl+R are intercepted, because a webview reload would
  throw away filters and sort.
- **The landing page** (ticket 020, `amd-ui-web/src/home.rs`): amdui
  always opens on it (`Ctx.view`: `View::Home`/`View::Board`). The `home`
  command returns `amd_ui_model::Home` (My work from `agile_md::my_work`,
  and the projects); `set_project_hidden` flips `ui.hidden-projects`.
  Opening a project or a work item reuses `set_tasks_dir` (which also
  remembers it as `ui.board`), then `open_ticket`. On Home the header
  hides New ticket, Admin and the filter bar, and ↻ reloads Home; on a
  board, ⌂ Home goes back. It's the shell documents (129), ideas (131) and
  collaboration (138) are meant to join. "You" is git's identity in each
  board's own repository, falling back to the global one.
- **Where a ticket is** (ticket 163): `Ctx::open_ticket` sets `ctx.focus`
  (repository, file); that card gets a ring and is scrolled into view once
  (`ctx.focus_scroll`, so a reload doesn't pull the board back), a folded
  column unfolds, and a backlog ticket in an epic or sprint opens that
  group's column. The editor says where it is under its title
  (`amd_ui_model::epic::where_it_is`: `acme-api › Backlog › Sprint s12
  (started)`, or `Doing · from epic checkout` from its `epic:` label).
- **Which project is current** (ticket 175): a sibling only counts as a
  project with a `tasks/backlog/` or a declared `project`
  (`board::is_project_board`), so an old v3 board left in a code repo is
  never offered or picked. The landing page's top bar prefers the project
  whose board was on screen (`ctx.board.tasks_dir`), else `Home.current`,
  so going Home keeps it; and `App` clears `ctx.document` whenever the
  shown board changes, so the reader never re-reads another project's
  document.
- **The running sprint in the header** (ticket 165): `ActiveSprints`,
  after the project name, shows a badge per started sprint
  (`Board.epics`, kind Sprint, `started`); a click goes to the board and
  opens its column (`ctx.open_epic`).
- **Which board a project opens**: `Board::discover()` from the launch directory; from
  outside any project (a desktop launcher starts in `$HOME`) it goes to the
  directory remembered as `ui.board` in `~/.config/agile-md/config`, which
  Preferences' "Board to open" sets.
- **Preferences vs Admin** (ticket 148): Preferences (`preferences.rs`) is
  personal and per machine (`~/.config/agile-md/config`: colour scheme,
  zoom, board to open), never committed. Admin (`admin.rs`) is the
  project's settings in the tickets `.settings`, committed for the team.
  Header: New ticket | Preferences | Admin.
- **Who sees Admin** (ticket 149, `crates/amd-lib/src/access.rs`): the
  project's `admins` list (`board_settings::admins`, emails or `Name
  <email>`), or admin/maintain on GitHub for the tickets repository's
  `origin` (`gh api repos/<owner>/<repo> --jq .permissions`; amd's git is
  SSH, which can't ask the API). Listed wins; with no list, GitHub decides;
  with no list and no answer (no `gh`, offline, not GitHub) Admin is open,
  with a note. amd-ui asks GitHub once per repository per session
  (`boards::admin_access`) and carries `Board.admin {allowed, note}`; the
  page hides the button and Admin shows the note. A convenience, not
  enforcement: the CLI's settings commands aren't gated. The colour scheme is `ui.theme`/
  `ui.accent` in the same file.
- **Documents** (ticket 129, `crates/amd-lib/src/documents.rs`):
  `View::Documents`, reached from each landing-page project row's
  **Documents / ADRs / Ideas / Shared** buttons (ticket 168), which set
  `ctx.doc_scope` (`amd_ui_model::DocScope`: own or shared repository, and
  a folder, `docs`, `adr` or `ideas`; `DocScope::holds`), so the sidebar
  lists only that folder until "Show everything". Buttons are greyed out
  when the repository isn't cloned (`HomeProject.documents`,
  `shared`/`shared_cloned`). The board has no tabs: a "Board" button
  shows only while documents are open. A documents repository is laid
  out `adr/adr/` (ADRs), `adr/rfc/` (RFCs), `docs/`, `ideas/`
  (`init.rs`'s `DOCUMENTS_FOLDERS`). `documents::sources` finds the
  project's `{project}-documents` (sibling first, else the registry's
  `find_near`) and the shared one the tickets `.settings` names
  (`shared-documents`, `amd shared-documents`, Admin's "Shared
  documents"); `list` walks markdown files, skipping dot-folders and not
  following links; `resolve`/`read`/`read_bytes` refuse anything outside
  the repository (`..`, absolute, a symlink out, `.git`) and files over
  `MAX_FILE`. `amd-ui`'s `documents.rs` renders with pulldown-cmark (its
  `html` feature is on only there): raw HTML becomes text, a link to
  another document becomes `#doc:<path>`, web/mail links stay and the page
  hands them to `open_external` (`xdg-open`/`open`, http(s)/mailto only),
  anything else becomes `#`, and relative images are inlined as data URLs
  (the CSP allows `data:` images only). **Documents are pushed straight
  to main, with git as their history** (agile-md-documents' ADR 0007,
  2026-10-10, superseding the PR review RFC 0003 asked for). **History
  and Restore** (ticket 169): `documents::history` (`git log --follow`, so
  across renames; each `Version` carries the path at that commit),
  `read_version` (`git show`, byte-exact, commit must be hex),
  `restore` (refuses a dirty file, a diverged clone and "nothing to
  restore"; fetches and fast-forwards first; commits only that path as
  `restore: <path> to <short> (<date>, <author>)`; pushes, replaying once
  with `rebase @{u}` if origin moved; a failed push keeps the commit and
  warns). Its own small sync, not `sync::catch_up`, which is about boards.
  amdui: `document_history` (20), `read_document_version`,
  `restore_document` (under the `writing` mutex); the reader's History
  button, `HistoryPanel`, and an old version's banner with Restore.
  `catch_up`/`push_or_replay` are shared with `amd import documents`
  (170). **Changing documents** (ticket 176, `documents_edit.rs`, amdui
  only; every change through `documents::commit_change`): read-only until
  **Edit** (rich text with the shared `rich_editor::Toolbar`, or Markdown;
  `rich::unsupported` — tables, images, HTML, footnotes, strikethrough —
  forces Markdown so nothing is flattened; frontmatter kept aside;
  `save` refuses a stale `version`); **New document** (`create_document`:
  `<slug>.md`, or the next `NNNN-` in a numbered folder, `# ADR NNNN:` /
  `# RFC NNNN:` in `adr/adr` / `adr/rfc`; opens for editing); **Archived**
  (`status: archived` in frontmatter, `with_archived`; muted, italic, a
  badge, a banner with Unarchive); **folders** (`folders` lists empty ones;
  `create_folder` keeps a `.gitkeep`; `delete_folder` only when empty);
  **move** (drag onto a folder heading, or Move…; `move_document` commits
  the pure `git mv` first so History follows, then rewrites links — other
  documents' links to it and its own relative links — in a second commit,
  both pushed). **Start page and search** (ticket 179): each project's own documents
  repository has one start page, `start-page = <path>` in `.agile-md` at
  its root (`documents_edit::{pinned, start_page, pin_start_page}`,
  committed like any document; README.md when nothing is pinned, or the
  pin is gone). Documents opens on it when nothing else is open; the
  reader has Pin as start page / Unpin and a Start page badge, and the
  sidebar a ⌂ Start page link. A move keeps the pin on the moved
  document. Search (`documents_edit::search`, `search_documents`) covers
  the own and shared repositories: titles and text, ignoring case, up to
  three lines a document, title matches first; a result opens the
  document and finds the text in it (`window.find`, started at the
  document's top). The CLI reads them too (`crates/amd-cli/src/docs.rs`, before
  `ensure_board()`): `amd docs start [--path]`, `amd docs search <text>`
  (`repo/path:line: text`), and `amd docs tree` — the link tree from the
  start page (`documents_edit::link_tree`: depth first, each page once,
  only markdown documents in the repository that exist). Pinning the start
  page stays in amdui; the CLI changes documents through the clone itself.
  **Your own pins** (ticket 232, `crates/amd-lib/src/doc_pins.rs`): per
  person and across machines, so in the project's own documents
  repository, one file each, `.agile-md-pins/<git user.email>` (other
  characters made `-`), one `<repository>/<path>` a line, own or shared
  repository. `doc_pins::set` commits `pin:`/`unpin:` through
  `commit_change` (re-reading the file after catching up; the file and
  folder go when empty); `moved` repoints everyone's pins in
  `move_document`'s link commit (a move in the shared repository leaves
  pins stale; they're listed as gone). amdui: **☆ Pin / ★ Pinned** in the
  reader, a Pinned section at the top of the sidebar (× unpins; a gone one
  is struck through), commands `my_pins`, `set_doc_pin`. CLI: `amd docs
  pins|pin|unpin`. Ticket 138's editing is covered here. The demo board has `acme-documents` and a shared
  `acme-handbook`.
- **The bottom panel** (ticket 185): on the board only, a one-line bar
  with **Terminal** and **AI** tabs; a click or Ctrl+` opens it, its top
  edge drags it taller, "Full height" fills the board's area (`room()`:
  window height less `main`'s top). All Rust, no JavaScript terminal:
  `crates/amd-ui/src/terminal.rs` runs the program on `alacritty_terminal`
  (its `tty` opens the pseudo-terminal, its `EventLoop` reads into a
  `Term`), and a frame thread per session sends the page the changed rows
  (`term.damage()`) at most every 16 ms as `terminal-frame` events, through
  the `Sink` trait (tests use a channel). Colours are resolved to CSS from
  the xterm palette; the default ones are left to the page's theme.
  Commands: `terminal_open(kind, cols, rows)`, `terminal_input`,
  `terminal_resize`, `terminal_scroll`, `terminal_close`; sessions live in
  `AppState.terminals` and are dropped (the program hung up) when the
  window is destroyed. The Terminal tab is `$SHELL -l`, the AI tab
  `$SHELL -lic "<ui.ai-command>"` (Preferences, `claude` by default), both
  in the project folder. The page (`panel.rs`, `term_view.rs`) keeps one
  signal per row, takes keys through a hidden textarea
  (`amd_ui_model::terminal::key_bytes`, `paste_bytes`), and F5, Ctrl+R and
  the zoom keys are left to the terminal while it has focus
  (`in_terminal`). A tab's program starts the first time it's shown, keeps
  running while the board is away, and ends when the project changes.
  Page panics now reach amdui's stderr (`page_panic`). Ticket 186 made
  the board available to whatever runs in the AI tab, through `amd mcp`. Since ticket 187 the panel is on **Board, Documents and
  Ideas** (`on_project`).
- **Speech to text** (ticket 187, `crates/amd-ui/src/speech/`): a 🎤 in
  the panel bar (hold to talk, or click to start and click to stop; or
  hold Ctrl+Shift+Space; Escape cancels) types what was said into the open
  tab, no Enter. **Nothing is downloaded by default**: Preferences →
  "Speech to text" offers tiny.en (42 MB, `lmz/candle-whisper`'s q80 GGUF)
  and base.en (290 MB, `openai/whisper-base.en` safetensors), fetched with
  `ureq` into `$XDG_DATA_HOME/agile-md/speech/<model>/` through `.part`
  files (`model.rs`); **Remove** deletes the folder (and empty parents),
  and "Use speech to text" (`ui.speech = off`) hides it and frees the
  model while keeping the files; `ui.speech-model` is the one in use.
  `audio.rs` records the default input with `cpal` (needs
  `libasound2-dev` to build on Linux; CI installs it) on a thread of its
  own; `dsp.rs` resamples to 16 kHz (our windowed sinc), cuts the burst a
  waking microphone makes (`trim_wake_up`: PipeWire suspends idle inputs
  and the first ~0.3 s measured RMS ~1.0, which Whisper heard as "you"),
  and computes the 80 Slaney mel filters. `whisper.rs` runs candle's
  Whisper (quantised or full), greedy, `[sot, notimestamps]` for the
  `.en` models, returns nothing when Whisper's no-speech probability is
  over 0.6, and decodes tokens itself from `tokenizer.json` (GPT-2 byte
  map). (candle-core itself depends on `tokenizers` with the C `onig`
  library on every desktop target, so it's built anyway, unused.) Debug
  builds leave dependencies' debug information out (`[profile.dev.package."*"]
  debug = false`): with candle, CI's runner ran out of disk linking amdui.
  The model loads on the first dictation
  (`AppState.speech`). Commands: `speech_status`, `speech_download`
  (`speech-progress` events), `speech_remove`, `set_speech_model`,
  `set_speech_enabled`, `speech_start`, `speech_stop`, `speech_cancel`.
  Ignored tests transcribe ALSA's voice clips
  (`AMD_SPEECH_WAV=/usr/share/sounds/alsa/Front_Center.wav
  AMD_SPEECH_EXPECT="front center"`) and record speaker playback.
- **Releases** build `amdui` too (ticket 085, `release.yml`). `install.sh`'s
  download path checks a downloaded amdui's libraries on Linux
  (`check_board_libraries`, via `ldd`) and names WebKitGTK when it's
  missing; its source build needs Trunk, the wasm target and WebKitGTK, and
  skips the board with a note without them.
- **Zoom** (ticket 024): Ctrl +/−/0 step through `amd_ui_model::zoom::STEPS`
  (50–200%); the page's `zoom_keys` sets `ctx.zoom` at once (so fast presses
  step on) and calls `set_zoom`, which applies Tauri's `WebviewWindow::set_zoom`
  and saves `ui.zoom` in `~/.config/agile-md/config`. `zoom` reapplies it at
  start. A real webview zoom, not CSS `zoom`, so `100vh` layouts still fit.
- **No `cursor-not-allowed`** (ticket 229): GTK on Wayland looks a CSS
  cursor up by its modern name only, and an older pointer theme (e.g.
  whiteglass) has no `not-allowed`, so every hover over a disabled button
  printed `Gdk-Message: Unable to load not-allowed from the cursor theme`
  in the terminal amdui was started from. Disabled buttons use
  `disabled:cursor-default`; their faded look already says they're off.
- WebKitGTK quirks met building it: the CSP needs `'wasm-unsafe-eval'` and
  `connect-src 'self' ipc: http://ipc.localhost`; native `<select>`s draw a
  white box (styled with `appearance-none`); `dragDropEnabled: false` so
  HTML5 drag and drop reaches the page.

## Conventions

- **Feature branch + PR for every real change** — `main` is not pushed to
  directly (the one exception was the initial full-source-tree restore after
  this repo's history was reset). Workflow-file changes need a
  `workflow`-scoped token.
- Co-authored commits use the bot identity, not the Anthropic no-reply:
  `Co-Authored-By: Claude <309050497+MrDClaudeBot@users.noreply.github.com>`.
