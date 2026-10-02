# agile-md

A tiny **filesystem Kanban** — tasks are markdown files moved between `todo/`,
`doing/` and `done/` directories. A single static binary plus `git`; no runtime,
no daemon, no database.

- **Status is the folder.** A task's column is simply which directory it's in.
- **Git is the audit trail.** Moving a task is a `git mv`, so `git log --follow`
  reconstructs exactly when it started and finished.
- **Tickets come from templates.** Task files are rendered from
  [MiniJinja](https://github.com/mitsuhiko/minijinja) templates, so the format
  is consistent by construction and yours to change.
- **The template is the form.** Leave an argument out in a terminal and `amd`
  asks — including the fields your own template declares.
- **One flexible ticket.** No ticket types to choose between: a `branch-type`
  gives it a branch (entering `todo` puts you on `bugfix/crash-on-save`), and
  without one it's a ticket with nothing to check out.
- **One command, any repo.** Install `amd` once on your PATH; it finds the
  board at `<repository-root>/tasks` from wherever you are.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/mrdoodles/agile-md/v4/install.sh | bash
```

Downloads the prebuilt binary for your platform (Linux/macOS, x86_64/aarch64)
into `/usr/local/bin` when it's writable, otherwise `~/bin`. Override with
`install.sh --dir <dir>`, pin with `--version vX.Y.Z`, or build locally with
`--from-source` (needs [Rust](https://rustup.rs)). From a clone:

```bash
cargo build --release        # target/release/{amd,amdui,amd-tui}
# a workspace: crates/amd-lib (the library), crates/amd-cli, crates/amd-ui
./install.sh                 # builds and installs
```

Three commands: **`amd`** is the command line, **`amdui`** is the desktop board,
**`amd-tui`** is the terminal board. `amd gui` opens the same window as `amdui`
and `amd tui` the same board as `amd-tui`.

## Use

In any git repository:

```bash
amd init                          # scaffold tasks/{todo,doing,done} here
amd new "Publish to Marketplace" --branch-type feature --assignee @me
amd board                         # show all columns (the default)
amd todo    1                     # backlog -> todo, and switch to feature/publish-to-marketplace
amd doing   1                     # todo -> doing (plain move, no branch effect)
amd done    1                     # doing -> done
amd todo                          # no ref: list what's in todo/ instead of moving anything
amd show    publish               # print a task (id or slug substring)
amd edit    1                     # open in $EDITOR
amd abandon 1                     # take it off the board, into tasks/archive/ — and
                                   # delete its branch too, if it's still empty
amd link    1 3                   # relate two tickets
amd tag     urgent                # list every ticket carrying a tag, across every column
amd todo    --tag urgent          # any listing form narrows to a tag: board, ls, a
                                   # destination verb, or an epic by name
amd lock    1                     # claim a ticket so someone else's edits and moves
                                   # are refused until you release it or it lapses
amd unlock  1                     # release your own lease
amd unlock  1 --force             # break someone else's — a separate, deliberate act
```

`amd` works from any subdirectory — it resolves the board from the repo root.
Set `AMD_DIR` to use a board directory other than `tasks`.

## What's on a ticket

One kind of ticket, with sensible defaults — fill in what applies and leave the
rest:

| Field         | | |
| ------------- | --- | --- |
| `id`          | automatic | `007`, from the board's counter |
| `title`       | required | names the file, and the branch |
| `assignee`    | optional | who's doing it; assignable later |
| `branch-type` | optional | `feature`, `bugfix`, `hotfix`, `release`, `chore` |
| `branch-name` | automatic | `<branch-type>/<title>`, empty without a branch type |
| `parent`      | optional | the ticket this sits under, any depth |
| `related`     | optional | ids of tickets this depends on |
| `tags`        | optional | free-form |
| `created`     | automatic | the date |

**The branch type is what makes a ticket a piece of code work**, and it's empty
by default. A ticket without one is still a ticket — a rota, an approval, a
renewal — it just has nothing to check out:

```bash
amd new "Renew the certificates"                    # no branch
amd new "Crash on save" --branch-type bugfix -a alex
amd todo 1      # moves it; "no branch type on this ticket"
amd todo 2      # moves it and switches to bugfix/crash-on-save
```

The branch types are the ones
[conventional-validator](https://github.com/mrdoodles/conventional-validator)
accepts, so a branch made from a ticket passes its check. `AMD_BRANCH_TYPES`
changes the list.

The branch name is written into the ticket at creation, so you can see it — and
edit it — before any work starts. Because the title becomes a branch, it's
**validated when you type it**: a title with nothing sluggable in it is rejected
rather than producing a ticket that can never start.

## Assignees

```bash
amd new "Add login" --assignee alex   # or -a @me for whoever git says you are
amd assign 3 sam                      # assign an existing ticket
amd assign 3                          # unassign it
```

Tickets are meant to be created now and assigned later, so `assignee` starts
empty. It shows on the board as `@sam`, and `amd ls --assignee` narrows to
one person. Who *created* a ticket isn't a field — `git log --follow` on the
file already knows.

## Related tickets

Tickets that depend on each other carry a `related` list — empty by default,
holding ids so a link survives a rename or a move between columns:

```bash
amd new "Session store" --related 1 --related add-logout   # ids or slugs
amd link 3 7                                               # link two that exist
amd link 3 7 --one-way                                     # …or just one end
```

The relation is symmetric: linking 3 to 7 records 7 on ticket 3 *and* 3 on
ticket 7, so it reads the same from either end and you can't forget the other
half. Refs are resolved when they're recorded, so a typo is an error rather
than a dangling link found later.

```markdown
related: [001,002]
```

Prose can still point at tickets with `[[NNN-slug]]` wikilinks; `related` is for
the dependency itself.

## Several repositories at once

**The list fills itself.** Every repository you run `amd` in is remembered, so
the boards you actually work on are all there next session — nothing to
maintain:

```bash
amd repos                     # what's been seen so far
amd repos add ../other-repo   # add one you haven't visited yet
amd repos remove other-repo   # by name or by path
```

`amd repos` works anywhere, including outside a repository, and asking what's on
the list doesn't add to it. A removed repository comes back the next time you
work in it; `AMD_NO_REGISTER=1` keeps the list entirely manual, and then removal
sticks.

On the desktop board a checkbox per repository says which are in view, and the
backlog shows one at a time. With more than one board in view each card says
which repository it came from, since ids restart at 001 in every repo.

The registry is a **list of repositories, and nothing else** — one path per line
in `~/.config/agile-md/repos`, written only when a repository is first seen:

```
# Repositories agile-md knows about, one path per line.
/Users/you/code/agile-md
/Users/you/code/other-repo
```

The tickets themselves are never copied into a store. They're markdown in each
repo, they change when you check out a branch or pull someone's work, and an
index of them would be wrong the moment it was written. Reading a board is a
directory listing and a few small files.

## Assignees

```bash
amd new "Add login" --assignee alex   # or -a @me for whoever git says you are
amd assign 3 sam                      # assign an existing ticket
amd assign 3                          # unassign it
```

The name lands in the ticket's frontmatter (`assignee: "sam"`) and shows on the
board as `@sam`, so it moves with the file and is visible in the diff.

## The desktop board

`amdui` opens the board in a window, across every registered repository at
once. It's the front end as its own command — launch it from a dock or a
runner, no subcommand to remember — and `amd gui` opens the same window if
you're already in a terminal:

```
BACKLOG (4)      TODO (3)         DOING (1)        DONE (7)
┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│ [017] 2pt    │ │ [001]   @sam │ │ [004]        │ │ [006]        │
│ Add repo…    │ │ Checkout…    │ │ Fix the…     │ │ Completions  │
└──────────────┘ └──────────────┘ └──────────────┘ └──────────────┘
```

- **Drag a card** up or down to reorder it, across to change its status. Both
  are written straight to disk — the move is a `git mv`, the order is an
  `order` key in the ticket.
- **Double click** a ticket to edit it: title, assignee, points and body. The
  body is rich text with an **MD/RT** toggle, click any line to type into it,
  checkboxes tick in place, and Enter continues a list.
- **Board / Backlog** switches views. The backlog groups tickets by epic and
  sprint, with **Add epic** and **Add sprint**.
- **Settings** holds the text size (standard, medium, large) and the colour
  scheme, both remembered in `~/.config/agile-md/config`.

A **sprint** takes only sized tickets — an unsized one would make its point
total a lie. A started sprint still takes tickets in and lets them out, because
teams do that and refusing only pushes it out of the tool's sight — but it will
not let a ticket be **archived** out of it. Move it out of the sprint first, so
the scope change is a `git mv` in the history rather than a disappearance.
An **epic** takes anything: it is where work goes before anyone has estimated
it.

A plain `cargo build` gives you both binaries. For the command line alone —
no windowing stack, which is what CI and scripts want —
`cargo build -p amd-cli --no-default-features`.

## Driving it from the command line

Everything the board does is a command, so a script — or an agent — can work a
board without opening a window. `--no-input` makes it never prompt.

```bash
amd new "Checkout revamp" --no-input   # lands in backlog/
amd set 001 points 5                   # points, epic, order, title
amd group epic checkout --description "the checkout flow"
amd group sprint sprint-1 --days 10
amd set 001 epic sprint-1              # refused unless the ticket is sized
amd group start sprint-1               # one way: no route back to pending
amd group                              # tickets and points per epic and sprint
amd group empty checkout               # every ticket in it, back to backlog/ at once
amd todo 001 --no-branch && amd doing 001 && amd done 001
```

The rules live in the board, not the window, so the command line gets them
too: a sprint refuses an unsized ticket, and refuses to have one archived
straight out of a started sprint.

`todo`/`doing` can each carry a WIP limit (`AMD_WIP_TODO`/`AMD_WIP_DOING`,
or the board's own `.settings`). A move into an already-full column is
refused, naming the limit; `--force` overrides it. The limit only ever
gates entry — moving a ticket *out* of an over-limit column always works.

`cargo build -p amd-cli --no-default-features` builds this without the desktop
board — the whole API, no windowing stack, which is what CI and scripts want.

## Shell completion

```bash
amd completions bash > /usr/local/etc/bash_completion.d/amd
amd completions zsh  > "${fpath[1]}/_amd"          # then: rm -f ~/.zcompdump; compinit
amd completions fish > ~/.config/fish/completions/amd.fish
```

`amd completions` with no argument works the shell out from `$SHELL`; elvish and
powershell are available too. The script goes to stdout and the install hint to
stderr, so redirecting gives you a clean file.

Completion covers values, not just names — `amd new --branch-type <TAB>` offers
the branch types this board accepts (including anything you added with
`AMD_BRANCH_TYPES`), `-T <TAB>` the templates, `amd ls <TAB>` the columns, and
`amd set <ref> <TAB>` the fields. Scripts are generated from the CLI itself, so
they can't drift from it.

## The board

In a terminal `amd board` draws with [richrs](https://crates.io/crates/richrs) —
tree guides, a colour per column, counts, and ids that open the ticket when
clicked:

```
TODO  (3)
├── [001] Checkout revamp  (feat)
│   └── [002] Guest checkout  (feat)
└── [004] Update the guide  (docs)
```

Piped or redirected output stays plain text (`    [002] Guest checkout  (feat)`,
indented by depth), so `amd board | grep` keeps working and log files don't fill
with escape sequences. `--plain` or `NO_COLOR=1` forces that everywhere.

## Why a directory instead of one big TODO.md?

A single file gets unwieldy once tasks have real content, and every edit is a
merge-conflict magnet. Separate files keep tasks self-describing, diffable and
independently movable, while the folders give an unambiguous board.

## Development

```bash
cargo test                    # unit tests (slugify, frontmatter, templates)
bash tests/test.sh            # end-to-end CLI spec against throwaway git repos
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

`amd` was pure bash through v3; v4 is the same tool in Rust, which is what
buys the templating. The CLI, the board layout and the file format are
unchanged — v3 boards work as-is.
