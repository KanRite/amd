<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="pages/logo/agile-md-dark.svg">
    <img src="pages/logo/agile-md-light.svg" alt="Agile-MD: plan, write, ideate, deliver" width="560">
  </picture>
</p>

# agile-md

**An open source, fully featured, multi-project agile management system for
individuals and teams.**

Your tickets, documents and ideas live in your own git repositories, as
markdown, right beside your work: not in someone else's cloud. agile-md works
well with AI, but AI is never a requirement; it's just as useful to a team or
a solo developer without it.

![The board: a running sprint beside the backlog, todo, doing and done](pages/screenshots/board.png)

## Main features

- **Kanban boards** across several projects and several boards, with
  optional sprints and automatic ticket prioritisation to help manage your
  workflow.
- **Customisable tickets** with an optional draft status, parent and child
  tickets nested to any depth, relationships, priorities and tags for
  filtering and sorting.
- **Your own board**: add, remove and rename columns, and set the workflow
  that suits your team.
- **Roadmaps and milestones** that set up their epics or sprints for you, to
  help manage larger projects and delivery dates.
- **Ideas**, to incubate new ideas and solve problems together, and turn them
  into backlog tickets, epics or sprints.
- **Documents** your team reads and writes, and can pin when they're reading
  or working from them: everything you need to be productive and informed.
- **Everything is backed by git**, giving you a full, intelligible audit
  trail and recovery when required.
- **Markdown in your repositories**, so your tickets, documents and ideas are
  accessible to your team, and to your AI.
- **A landing page** showing your priorities across all your projects, with
  statistics and navigation.
- **A desktop app** with rich text editing, and a **command line** for those
  who prefer it.
- **A built-in terminal**, and optional AI integration.
- **Optional speech to text**, run entirely on your machine.
- **Free to use:** no servers, no hidden costs. Your data is where you need
  it, and in your control.

## In more detail

### Tickets

- **One flexible ticket**, rendered from templates each project can change.
- **Fields:** priority, story points (1, 2, 3, 5, 8, 13), assignee from the
  project's users, tags and the repository it's about.
- **Typed links:** requires, related, parent and child, clone, supersedes,
  each written on both tickets and changeable later. A tree draws what each
  ticket requires and relates to, and cards show how many links they have.
- **Save and New:** found a ticket depends on one that doesn't exist yet?
  Save it and make the missing one, already linked.
- **Who created it** is recorded; **who last edited it** comes from git.
- **Tags** are uppercase and can be limited to the project's own list; DRAFT
  and TRIAGE are built in.
- **Drafts** can't leave the backlog until they're marked ready.
- **Branches:** a ticket with a branch type gets its own git branch when it
  moves to todo.
- **Search and filters** by tag, assignee, repository and text.

### Workflow rules

- **One column at a time:** backlog → todo → doing → done, unless the team
  allows other moves.
- **Your own columns:** add Triage, Review, Released and the like, rename,
  reorder and remove them; each has a kind that decides what it does, and
  reset puts the standard four back.
- **Done waits** for the tickets a ticket requires and for its children.
- **A WIP limit on doing** (2 unless changed), with a deliberate override.
- **Locks:** a ticket is yours while it's in doing and is released when it
  leaves; someone else's lock is only broken on purpose.
- **Every change is committed and pushed**, and boards stay in step: amd
  catches up before a change, replays it if someone pushed first, and warns
  when many tickets vanish or history is rewritten.

### Planning

- **Epics and sprints**, both optional; a started sprint takes nothing more
  and gives nothing back, and can't start while it needs a ticket outside it.
- **Auto priority** (optional): a ticket is at least as urgent as what
  requires it, and the **Automatic** sort shows what's needed first.
- **Roadmaps:** one per board, made of milestones that create its epics or
  sprints, with working days, time left and whether each is on track. A
  milestone's start and due dates can be changed later.
- **My work:** your todo and doing tickets across every project on your
  machine, and work to pick up when you have none.

### Documents and ideas

- **A documents repository per project** for ADRs, RFCs and docs, plus an
  optional one every project shares. Changes go straight to main, with
  history and restore.
- **Write in amdui:** rich text or markdown, folders, moves that keep links
  working, archiving, a pinned start page and search, and your own pinned
  documents, first in your list on every machine.
- **Ideas** before they're tickets: everyone votes up to ±3 and adds notes,
  and an idea becomes a backlog ticket in one click, or (for the project's
  admins) an epic or a sprint with the idea's write-up as its description
  (`amd idea ticket <idea> --as epic|sprint`).

### Teams and projects

- **`amd init`** creates a project's repositories on GitHub; **`amd join`**
  gives a teammate the project and adds them as a user.
- **Several repositories per project**, and several projects side by side;
  a new repository can be made from the ticket that needs it.
- **Admin** settings are committed for the team; **Preferences** are your
  own, on your machine.
- **Archive a project** to make it read-only, on GitHub too if you like.

### Bringing work in, and getting it back

- **GitHub issues** arrive as TRIAGE tickets, each one only once.
- **Import** tickets from another board and documents from a folder.
- **`amd recover`** brings back deleted or abandoned tickets; **`amd
  renumber`** fixes duplicate numbers.

### The desktop board

| | |
|---|---|
| ![A ticket open in the editor](pages/screenshots/ticket.png) | ![A roadmap and its milestones](pages/screenshots/roadmap.png) |
| ![Documents, with an ADR open](pages/screenshots/documents.png) | ![Ideas, best voted first](pages/screenshots/ideas.png) |
| ![The landing page: My work and the projects](pages/screenshots/home.png) | ![The terminal in the bottom panel](pages/screenshots/panel.png) |


- **Drag and drop**, with the rules shown while you drag.
- **A landing page** with My work and every project.
- **Live:** changes made anywhere (the command line, an editor, a pull) show
  within a second.
- **A bottom panel** with a terminal and an AI tab running your own AI tool.
- **Speech to text**, optional and entirely on your machine.
- **An Editor button**, themes, accent colours and zoom.
- **Update check:** it tells you when a newer release is out.

### AI, if you want it

- **`amd mcp`** lets any MCP-capable AI tool (Claude Code, Copilot, Cursor,
  Codex, Gemini CLI) read the board, documents, ideas and what's on screen,
  and change tickets under the same rules, each change committed. It never
  abandons tickets, breaks locks or changes settings.
- **Nothing built in:** amd holds no API keys, and nothing runs unless your
  AI tool starts it.

### Command line

- **Everything the board does is a command**, safe in scripts and CI: it
  never prompts without a terminal.
- **Shell completion** for bash, zsh and fish, and `amd log` for a board's
  history.

## Install

Runs on **Linux and macOS**, and needs **git 2.29 or later** (`git --version`)
on your `PATH`.

```bash
curl -fsSL https://raw.githubusercontent.com/agile-md/amd/v4/install.sh | bash
```

Downloads the newest release's binaries for your platform (Linux/macOS,
x86_64/aarch64), `amd` and the desktop board `amdui`, into `/usr/local/bin`
when it's writable, otherwise `~/bin`. Override with
`install.sh --dir <dir>`, pin with `--version vX.Y.Z`, or build locally with
`--from-source` (needs [Rust](https://rustup.rs)). From a clone:

```bash
(cd crates/amd-ui-web && trunk build --release)   # the board's pages, first
cargo build --release        # target/release/{amd,amdui}
# a workspace: crates/amd-lib (the library), crates/amd-cli, and the desktop
# board — crates/amd-ui (Tauri), crates/amd-ui-web (Leptos), crates/amd-ui-model
./install.sh                 # builds and installs
```

On Linux, installing from a clone also adds the board to your application
menu: a launcher entry and the agile-md icon in `~/.local/share`, which is
where the dock and top bar take the icon from.

The desktop board needs [Trunk](https://trunkrs.dev) and the
`wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`)
to build its pages, and on Linux WebKitGTK and ALSA (`libwebkit2gtk-4.1-dev
libasound2-dev`; ALSA is for speech to text's microphone). For the
command line alone, `cargo build --release -p amd-cli` needs none of them.

Two commands: **`amd`** is the command line and **`amdui`** is the desktop
board. `amd gui` opens the same window as `amdui`.

## Use

Start a project, or join one:

```bash
amd init acme                     # make acme-tickets and acme-documents on GitHub and clone them here
amd join acme                     # …or join a project someone else made
amd init acme --local-only        # …or keep the board in the repository you're in (tasks/)
```

Then, from any of its repositories (or the folder holding them):

```bash
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
amd mine                          # your todo and doing tickets, across every project on this machine
amd recover                       # tickets deleted or abandoned: who removed each, and when
amd recover 1                     # bring one back into the backlog, exactly as it was
amd renumber                      # two tickets with the same number: shows the fix
amd renumber --apply              # ...and makes it, in one commit
amd link    1 3                   # relate two tickets
amd tag     urgent                # list every ticket carrying a tag, across every column
amd tags                          # the project's tag list, with how many tickets carry each
amd tags add future               # add a tag (saved as FUTURE); `amd tags remove` takes an unused one off
amd new     "Idea" --draft        # a draft: tagged DRAFT, it stays in the backlog
amd ready   1                     # take it out of draft (`amd draft 1` puts it back)
amd todo    --tag urgent          # any listing form narrows to a tag: board, a
                                   # destination verb, or an epic by name
amd lock    1                     # claim a ticket so someone else's edits and moves
                                   # are refused until you release it or it lapses
amd unlock  1                     # release your own lease
amd unlock  1 --force             # break someone else's — a separate, deliberate act
```

`amd` works from any subdirectory — it resolves the board from the repo root,
and from a code repository it finds the project's tickets repository beside
it. Set `AMD_DIR` to use a board directory other than `tasks`.

## What's on a ticket

One kind of ticket, with sensible defaults — fill in what applies and leave the
rest:

| Field         | | |
| ------------- | --- | --- |
| `id`          | automatic | `007`, from the board's counter |
| `title`       | required | names the file, and the branch |
| `assignee`    | optional | who's doing it; assignable later |
| `priority`    | default `low` | `low`, `medium` or `high` |
| `points`      | optional | its size: 1, 2, 3, 5, 8 or 13 |
| `repository`  | optional | the repository it's about, where its branch is made |
| `branch-type` | optional | `feature`, `bugfix`, `hotfix`, `release`, `chore` |
| `branch-name` | automatic | `<branch-type>/<title>`, empty without a branch type |
| `parent`      | optional | the ticket this sits under, any depth |
| `related`     | optional | ids of tickets this relates to |
| `links`       | optional | typed links: `requires`, `supersedes` and the like |
| `tags`        | optional | always UPPERCASE; from the project's tag list (DRAFT and TRIAGE are always on it) |
| `created`     | automatic | the date |
| `created-by`  | automatic | who made it, `Name <email>`; never changes |

**Who to ask.** `amd show` and the desktop board's editor say who created a
ticket and who last edited it: "created by Priya Shah on 2026-10-02; last
edited by Sam Lee on 2026-10-08". Last edited comes from git history (a move
between columns doesn't count), so an edit made in any editor is credited
to the right person. A ticket made before `created-by` existed is credited
from git, and gets the field the next time it's saved.

**Tags** are always UPPERCASE, so they stand out. Each project keeps a tag
list (Admin in the desktop board, or `amd tags`), saved in the tickets
repository's `.settings`. Every list starts with the built-in DRAFT and
TRIAGE; add the project's own with `amd tags add`. The editor offers the
list as a sorted dropdown, and a tag that isn't on it is refused: "Prevent
arbitrary tags" is on by default, so a new project allows only DRAFT and
TRIAGE until you add more. Turn it off (`amd tags only off`) to allow other tags, which `amd
new` then only warns about and the editor shows dashed. A tag can only be removed from
the list once no ticket carries it. Matching ignores case, so a ticket written
before tags were uppercase still matches.

**Drafts.** A ticket tagged **DRAFT** is a draft: it can't leave the
backlog, or go into a sprint, until the tag is taken off. It can still be
edited, assigned, sized and filed into an epic, or abandoned. Make one with
`amd new --draft`, `amd draft <ref>` or "Save as draft" in the desktop
board; `amd ready <ref>` (or the editor's **Mark ready**) takes it out of
draft, and `amd tag DRAFT` lists them. DRAFT is built in: it's allowed even
when only listed tags are, and it can't be removed from the list.

**The branch type is what makes a ticket a piece of code work**, and it's empty
by default. A ticket without one is still a ticket — a rota, an approval, a
renewal — it just has nothing to check out:

```bash
amd new "Renew the certificates"                    # no branch
amd new "Crash on save" --branch-type bugfix -a alex
amd todo 1      # moves it; "no branch type on this ticket"
amd todo 2      # moves it and switches to bugfix/crash-on-save
```

The branch is made in the repository the ticket names (`--repository`, or
`amd set <ref> repository <name>`). On a board in its own `*-tickets`
repository, the layout `amd init <project>` makes, a ticket that wants a
branch but names no repository is refused on entering todo rather than
branching the tickets repository; `--no-branch` moves it anyway.

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
empty. The name lands in the ticket's frontmatter (`assignee: "sam"`), so it
moves with the file and shows in the diff; the board shows it as `@sam`, and
the desktop board's User filter narrows to one person. In a terminal, a bare
`amd assign 3` offers the project's users to pick from. Who *created* a
ticket is recorded separately, as `created-by`.

## Related tickets

Tickets that belong together carry a `related` list — empty by default,
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

A link can say more than "related": `--type requires`, `parent-of`,
`child-of`, `clone-of`, `supersedes` and their opposites, each recorded the
other way round on the other ticket. **A ticket can't move to done until the
tickets it requires and its children are done**, and nothing overrides that.
If the link was wrong, change it (`amd link 3 7 --replace requires --type
relates-to`) or remove it (`--remove`). On the desktop board a card that's
waiting says "waits on N", and each link in the editor has a type picker.

`amd new "Checkout" --requires 3` and `amd set 12 requires 3,7` set what a
ticket requires directly (`set` makes the list exactly that; empty clears
it). **A sprint can't start while one of its tickets requires a ticket
that's neither in the sprint nor done**: `amd group start` names each one
(`s12 can't start: 003 requires 010 (backlog)`), and so does the sprint's
Start button on the desktop board. Outside sprints nothing more is enforced.

`amd tree 12` draws a ticket's family, what it requires (solid) and what
it relates to (dashed), saying where each ticket is and what's still
waited on; `amd tree` draws every family on the board. Done tickets are
left out unless you add `--done` (`amd tree 12 --done`):

```
001 Alpha · backlog  waits on 002
├──── 002 Beta · backlog
│     └──── 003 Gamma · done ✓
└ ─ ─ 004 Delta · backlog
```

That's `amd tree 4 --done`. Tickets in a sprint that has started aren't
expanded: a link to one shows it as "in started sprint s12".

On the desktop board, a card that requires or relates to others has a small
tree button that opens the same tree, with that ticket highlighted; click
any ticket in it to go to its card. Done tickets are left out until you
tick "Include done". A sprint that can't start lists what's
missing, each with "Show the tree".
Each card also says how many tickets it's linked to (beside the tree icon,
or a link icon when it has no tree), and a ticket linked to nothing at all
shows a small blue circle, so loose tickets are easy to spot.

**Auto priority** (off unless switched on: `amd auto-priority on`, or
Admin → Auto priority) makes a ticket at least as urgent as the most urgent
ticket that requires it, down a chain, so the priorities you set on your
goals flow to the work they wait on. A raised ticket records why
(`priority-from`, `priority-own`; `amd show` says it, and its card shows
`↑ high`), and switching it off puts them back. With **allow override** on
(the default, `amd priority-override`), a priority you set by hand pins that
ticket; `amd set 12 priority auto` (or "auto" in the editor) hands it back.
With it off, a hand-set priority is only a floor.

Auto priority only passes urgency down: if the ticket at the top of a chain
is low, everything it needs stays low. To see the work in the order it has
to be done, pick **Automatic: what's needed first** in the desktop board's
Sort (the default on a board with auto priority on): within each column, a
ticket comes after the tickets it requires, and otherwise highest priority
first, then oldest.

```markdown
related: [001,002]
```

Prose can still point at tickets with `[[NNN-slug]]` wikilinks. For a real
dependency, use `requires`: that's the one done and sprints wait for.

## Several repositories at once

A project's tickets live on one board, in `{project}-tickets`, and a ticket
says which repository it's about (`--repository`, or `amd set <ref>
repository <name>`), which is also where its branch is made. The
repositories a project spans are declared in its `.settings`, so the whole
team shares the list:

```bash
amd register ../acme-api      # add a repository to the project (and note where it's cloned here)
amd clone                     # the project's repositories this machine hasn't got
amd clone acme-api            # clone one beside the others, and note it
amd deregister acme-api       # take one off the project
```

Where each repository is cloned is kept per machine, one path per line in
`~/.config/agile-md/repos`; which repositories the project has is shared.
A repository can also have a board of its own (a `tasks/` in it); its
tickets show on the project's board too, each card saying which repository
it came from, since ids restart at 001 on every board. The desktop board's
Repository filter narrows it to one.

**A repository the project hasn't got yet** can be made from the ticket that
needs it: the editor's Repository list has **＋ New repository…** (for the
project's admins), and the command line has `amd repo new`:

```bash
amd repo new acme-payments --for 12          # make it, clone it, add it to the project, set ticket 12
amd repo new acme-payments --public --description "Card payments"
```

It's made under the tickets repository's owner on GitHub (through `gh`),
private or public as the tickets repository is unless you say otherwise,
with a starter README (`--no-readme` leaves it out) and `main` protected
where GitHub allows. A repository of that name that's already there is
connected instead. It's cloned beside the tickets repository, added to the
project for everyone, and the ticket gets it as its repository with a dated
note saying so, all in one commit.

The tickets themselves are never copied into a store. They're markdown in each
repo, they change when you check out a branch or pull someone's work, and an
index of them would be wrong the moment it was written. Reading a board is a
directory listing and a few small files.

## My work, across every project

`amd mine` (and the desktop board's landing page) lists the tickets
assigned to you in todo and doing, across
every project on this machine (everything `amd init`, `amd join` or `amd
register` has registered), highest priority first, then oldest:

```
doing  high     3 pts  acme              017  Fix the basket total rounding
todo   high     2 pts  acme              015  Screen reader labels on the product page
doing  low      1 pts  fantastic-tools   004  Write the returns policy page
```

"You" is git's identity in each project's own repository (else your global
one), matched against that project's users list for the name the team
knows you by. `--sort age` or `--sort points` changes the
order, and `--json` is for scripts. `amd mine --hide <project>` leaves a
project out on this machine (`--unhide` puts it back, `--all` shows
everything); scratch boards under the temporary directory are never
listed. It reads each board as this machine last saw it.

With nothing in todo or doing, it offers work to pick instead (never a
draft, or a ticket locked by someone else). With unassigned tickets
included (the landing page's "Include unassigned tickets" checkbox, on
by default; `amd mine --unassigned`/`--no-unassigned` set it), that's
unassigned todo and doing tickets first; failing those, backlog tickets
that are unassigned or yours: every high-priority one there is, else
every medium, else every low, oldest first. Without, only backlog tickets
assigned to you. Hidden projects are searched too, and one with work to
pick is shown again.

## Bringing in existing work

Both imports work the same way: **stand where the things should go, and
name where they come from.** Nothing is ever overwritten: anything left out
is printed and appended to `import.log` in the folder you ran it from (it's
kept out of git, so it's never committed). `--dry-run` shows what would
happen and writes nothing.

**Tickets**: from this board's `tasks/` directory, name another board's
`tasks/` directory (the one holding `backlog/`, `todo/`, `doing/` and
`done/`):

```
cd acme-tickets/tasks
amd import tickets ../../old-project/tasks --dry-run
amd import tickets ../../old-project/tasks     # a bare `amd import <PATH>` is the same
```

Each ticket keeps its column, gets a fresh number, and keeps its parent and
related links within the batch. A ticket whose title is already on this
board is left out (and logged); two same-titled tickets within the batch are
both brought in, tagged `DUPLICATE`. The import is one commit.

**Documents**: from the folder of a documents repository the files should go
into, name the folder to copy:

```
cd acme-documents/docs
amd import documents ~/old-notes --dry-run
amd import documents ~/old-notes               # commit on main and push
```

The folder is mirrored as it is: every markdown file, and every picture or
other file a document links to; hidden files and folders are left out.
Documents are pushed straight to main, with git as their history, so the
import catches up with origin, commits on the main branch and pushes, and
amdui's History can undo any of it. It refuses a clone with uncommitted
changes, one that isn't on its main branch, and one that has diverged.

**GitHub issues** come in by themselves: when amdui's landing page opens, it
pulls in the open issues of each project's repositories on GitHub (at most
every five minutes per project), tagged `TRIAGE`, with a link back to the
issue and its text, as one commit. A label becomes a tag when it's on the
project's tag list; any other is named in the ticket's notes. An issue is
only ever imported once, so abandoning a `TRIAGE` ticket turns its issue down
for good. New tickets start at the project's triage priority (`amd
triage-priority medium`, or Admin). Admin's "GitHub issues" switch, or `amd
issues off`, stops the automatic pull; `amd issues` checks now (`--repo
owner/repo` and `--label` for a one-off, `--dry-run` to preview). It reads
GitHub with `gh`.

## Working together, and getting tickets back

Every change amd makes is committed and pushed, and before each command it
catches up with the remote: it fast-forwards when you're behind, pushes
anything you made offline, and refuses a change on a clone that has
diverged rather than write on stale state.

Nothing a commit has seen is lost. `amd recover` lists every ticket that was
deleted or abandoned, with who removed it and when, and brings any of them
back exactly as it was. If catching up would remove five or more tickets at
once, amd says so (how many, which commits, by whom) and points at `amd
recover`. If someone force-pushes and rewrites the remote's history, amd
names it as that and changes nothing, so your clone, which still holds the
old history, is there to put it back. Branch protection on `main` stops a
force push in the first place.

## Who can change the project's settings

The project's settings (the WIP limit, lock length, tags, admins,
repositories and the rest) live in the tickets repository's `.settings`,
shared with the whole team. The ticket number counter is kept apart, in
`.next-id`, so `.settings` changes only when someone changes a setting.

**What amd does on its own.** The desktop board shows **Admin** only to the
project's admins: people on the `admins` list in `.settings`
(`admins = Alex Morgan <alex@example.com>, sam@example.com`), or with admin
or maintain access to the tickets repository on GitHub (amd asks through the
`gh` CLI when it's installed). That keeps settings out of everyone else's
way, but it isn't a lock: anyone who can push to the tickets repository can
still edit `.settings` by hand.

**Enforcing it on GitHub.** To make GitHub itself refuse settings changes
from anyone but admins, add a **push ruleset** to the tickets repository.
GitHub then rejects any push that touches those files unless the pusher is
on the bypass list, whatever tool they use. Ticket moves and new tickets
don't touch them, so they're unaffected.

1. In the tickets repository: **Settings → Rules → Rulesets → New ruleset →
   New push ruleset**.
2. Add the rule **Restrict file paths** with `.settings` and `.github/**`.
3. Under **Bypass list**, add **Repository admin** and **Maintain**.
4. Set enforcement to **Active** and save.

GitHub only offers push rulesets on **private repositories on a paid plan**.
They don't exist for public repositories at all (GitHub refuses with "Source
public repos cannot have push rules"), and a private repository on the free
plan gets "Upgrade to GitHub Pro" instead. Check GitHub's current rulesets
documentation for which plans include them.

When a push is refused, amd keeps the change in your clone and says the push
failed; nothing is lost. Ask an admin to make the change, or undo yours with
`git reset --hard origin/main` in the tickets repository once you've checked
nothing else is waiting to be pushed.

## The desktop board

`amdui` opens a project's board in a window, with the tickets of all its
repositories. It's the front end as its own command — launch it from a dock or a
runner, no subcommand to remember — and `amd gui` opens the same window if
you're already in a terminal. Launched from outside a project, it opens the
board it showed last. It is a Tauri window over a Leptos page, and every
change it makes goes through the same library as `amd`, so the two never
disagree.

- **It opens on a landing page**: **My work** (your todo and doing tickets
  across every project on this machine, highest priority first; click one
  to open it on its board) and **Projects** (click one to open its board,
  or **Documents** for its documents; **Hide** leaves it out of My work on
  this machine). **⌂ Home** in a board's header comes back. "You" is git's
  identity in each project's own repository, so a different email per
  organisation works.
- **Documents**: a project's documents, from the landing page's top bar
  (Documents, ADRs, Ideas, Shared). The left lists the markdown files in
  the project's `{project}-documents` repository by folder (`adr/adr`,
  `adr/rfc`, `docs`, `ideas`), titled by their first heading, and the right
  reads one: tables, code, task lists and pictures from the repository.
  Links between documents open in the reader; web links open in your
  browser. **History** on any document lists its versions from git; open
  one to read it, and **Restore this version** puts it back as a new
  commit, pushed, so a restore can itself be undone. A document is
  read-only until **Edit**, which edits it as rich text (or as Markdown,
  when it has tables or images that rich text can't keep). **＋doc** on a
  folder makes a new document (numbered, in `adr/adr` and `adr/rfc`);
  **＋folder** makes a folder, and an empty one can be deleted. **Archive**
  marks a document as kept for the record: it shows muted, with a badge.
  Drag a document onto a folder, or use **Move…**, to move it; links to it
  are rewritten so they keep working. **☆ Pin** puts a document you're
  writing or working from at the top of your own list, under Pinned; pins
  are yours, kept in the documents repository so they follow you to every
  machine (`amd docs pins`, `amd docs pin <repository>/<path>`, `amd docs
  unpin`). Every change is pushed straight to
  main rather than reviewed in pull requests. A
  documents repository the whole organisation shares (standards, the
  handbook) can sit beside it: name it in Admin, or with
  `amd shared-documents <repository>`, and clone it next to the project's
  repositories. Raw HTML in a document is shown as text, never run.
- **A Backlogs column** of sprints, then epics, beside the board's columns. Drag
  a card to the next column, or into and out of an epic or sprint; the move is
  a `git mv`, committed and pushed like `amd`'s. Moving into doing
  locks the ticket for you, and moving it out again unlocks it: a ticket is
  locked while it's being worked on.
- **Click a ticket** to edit it: its fields as a form (title, assignee, branch
  type, tags, points, priority), its relationships (parent, children,
  related, and tickets that link to it), each a click away, and its body as
  rich text, with a switch to the raw markdown. Lock, renew, unlock — or break someone else's lock, after
  a confirmation — from the same window. **Save and New** saves the ticket
  and opens New ticket linked back to it ("012 requires this" unless you
  pick related or no link), for when you find it depends on a ticket that
  doesn't exist yet.
- **Filters** across the top: repository, person, a search, the sort order,
  and Reset.
- **New ticket**, **+ Sprint**, **+ Epic**; open one from its box to start
  a sprint, rename it, or delete it once everything in it is done.
- **Preferences**, your own settings on this machine, in
  `~/.config/agile-md/config`: the colour scheme (light, dark or the
  system's, with an accent colour), the zoom, the board to open when amdui
  starts outside a project, the AI tab's command, speech to text, and your
  editor.
- **Admin**, the project's settings, shared with the team through the
  tickets repository: auto-commit, tags, the columns and how tickets move
  between them, WIP limits, how long a lock lasts, auto priority, GitHub
  issues, the shared documents, renaming epics and sprints, archiving the
  project, and the ticket template (with a preview). It's shown to the project's admins: anyone on its `admins` list
  (`admins = Alex Morgan <alex@example.com>, sam@example.com` in the tickets
  `.settings`), or with admin or maintain access to the tickets repository
  on GitHub (asked through the `gh` CLI). When amd can't tell (no list, no
  `gh`), Admin stays open. It's a convenience, not a lock: anyone who can
  push to the tickets repository can still change `.settings`.
- **Zoom** with **Ctrl +** and **Ctrl −**, and **Ctrl 0** back to 100%; the
  size is remembered on this machine (`ui.zoom`). F5 or Ctrl+R refreshes the
  board without losing your filters.

A **sprint** takes only sized tickets — an unsized one would make its point
total a lie. Once **started** it is a commitment: it takes nothing more and
gives nothing back, and its tickets only move on, to todo. An **epic** takes
anything: it is where work goes before anyone has estimated it. The loose
backlog is just the default epic — a ticket in any epic or sprint can move on
to todo, keeping its epic, and sent back from todo it returns to it (unless
that is a sprint that has started).

`cargo build -p amd-cli` gives you the command line alone — no window or web
stack, which is what CI and scripts want.

## Driving it from the command line

Everything the board does is a command, so a script — or an agent — can work a
board without opening a window. `--no-input` makes it never prompt.

```bash
amd new "Checkout revamp" --no-input   # lands in backlog/
amd set 001 points 5                   # points, epic, order, title, priority
amd set 001 priority high              # low (the default), medium or high
amd group epic checkout --description "the checkout flow"
amd group sprint sprint-1 --days 10
amd set 001 epic sprint-1              # refused unless the ticket is sized
amd group start sprint-1               # one way: no route back to pending
amd group                              # tickets and points per epic and sprint
amd group empty checkout               # every ticket in it, back to backlog/ at once
amd group rename checkout payments     # the folder and every ticket's epic
amd group delete payments              # once every ticket in it is done (or moved out)
amd todo 001 --no-branch && amd doing 001 && amd done 001
```

The rules live in the board, not the window, so the command line gets them
too: tickets move one column at a time (unless the team allows other moves),
a sprint refuses an unsized ticket, and a started sprint takes nothing more
and gives nothing back.

`doing` carries a WIP limit: 2 unless changed (`amd wip-limit <n>`,
`AMD_WIP_LIMIT`, or the board's own `.settings`; 0 turns it off). Todo is
never limited, so work can't get stuck there. A move into a full doing is
refused, naming the limit; `--force` overrides it. The limit only ever
gates entry — moving a ticket *out* of doing always works.

`cargo build -p amd-cli` builds this without the desktop board — the whole
API, no window or web stack, which is what CI and scripts want.

## Using AI with agile-md

Bring your own AI tool, or none. `amd mcp` is an
[MCP](https://modelcontextprotocol.io) server: any AI tool that speaks it
(Claude Code, Copilot in VS Code, Cursor, Codex, Gemini CLI and others)
can work on the board through amd itself rather than by editing files.
Connect yours once:

```bash
amd mcp setup              # what to add for each tool
claude mcp add --scope user agile-md -- amd mcp   # e.g. Claude Code
```

Then ask it things like "what am I looking at?", "what should I pick up
next?", "make a ticket for this bug" or "move this to doing". Its tools:

- **Read:** what amdui is showing (the open ticket, document or idea, and
  the board's filters), the board, a ticket, ticket search, My work, the
  documents (search, a document, the start page and its tree) and ideas.
- **Change tickets:** create, move, edit, assign, add a note, and file into
  an epic or sprint. Each goes through the same rules as the board: one
  column at a time, the WIP limit, drafts, started sprints, locks. Each is
  committed and pushed as `amd` would, so git has the record.
- **Never:** abandoning a ticket, breaking someone's lock, settings, or
  branches. Those stay with you.

Your AI tool asks before running a change, as it does for its other tools.
Nothing runs unless your AI tool starts `amd mcp`; amd holds no API keys
and talks to no AI service itself. For "this" to work, amdui leaves a note
of what it's showing (names and paths only) in
`~/.local/state/agile-md/screen/`; the AI reads content through the tools,
which your AI tool lists as it goes.

amdui's **AI tab** runs your AI tool's command line (Preferences → AI
command, `claude` by default) in the project folder, so with `amd mcp`
connected it can see what's on the board beside it.

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
`AMD_BRANCH_TYPES`), `-T <TAB>` the templates, and
`amd set <ref> <TAB>` the fields. Scripts are generated from the CLI itself, so
they can't drift from it.

## The board

In a terminal `amd board` draws with [richrs](https://crates.io/crates/richrs) —
tree guides, a colour per column, counts, and ids that open the ticket when
clicked:

```
TODO  (3)
├── [001] Checkout revamp  (feature @Priya Shah)
│   └── [002] Guest checkout  (feature)
└── [004] Update the guide
```

Piped or redirected output stays plain text (`    [002] Guest checkout  (feature)`,
indented by depth), so `amd board | grep` keeps working and log files don't fill
with escape sequences. `--plain` or `NO_COLOR=1` forces that everywhere.

## Your own columns

A board starts with Backlog, To do, Doing and Done. A team can add its own
(Triage, Review, Blocked, Released…), rename any column, move columns along
and take its own ones away again, from the CLI or the desktop board's Admin →
Columns. Each column has a **kind** that decides what it does:

| Kind | What it does |
|---|---|
| backlog | work waiting; epics and sprints live in the Backlog itself |
| todo | entering it makes the ticket's branch |
| doing | the ticket is locked while it's there, and the column has a WIP limit |
| done | a ticket can't go in while what it requires isn't done; anything here counts as done |

```bash
amd column add Review --kind doing --after doing
amd column wip review 3
amd column add Triage --kind backlog --before backlog
amd column rename todo Ready
amd review 12                  # or: amd move 12 review
amd column remove review --move-to doing
amd column                     # name, folder, kind, WIP limit and count of each
```

**Moves and where new work arrives.** Tickets move one column at a time,
either way, unless the team says otherwise: `amd column allow triage todo`
adds an extra one-way move beside that, and `amd column flow any` lets a
ticket go from any column to any other. Whatever the flow, a draft stays in
the backlog, a started sprint's tickets only move on to todo, done waits for
what a ticket requires, and WIP limits and locks still apply. `amd column
landing triage` makes new tickets (from `amd new`, the desktop board, an AI
tool or an imported GitHub issue) arrive in Triage; one filed straight into
an epic or sprint still goes to the Backlog. `amd column reset` puts the
board back to Backlog, To do, Doing and Done: each ticket in an added column
moves to the standard column of the same kind, and the names, flow, extra
moves, landing column and column WIP limits go back to standard, in one
commit.

A rename only changes the name: the
folder, and so every ticket's history, stays where it is. The four standard
columns can be renamed and moved but never removed, and every kind always has
at least one column. The columns are kept in the tickets repository's
`.settings` (`columns = …`) and committed, so the whole team gets the same
board; everyone needs amd 4.1 or later to see tickets in an added column.

## Why a directory instead of one big TODO.md?

A single file gets unwieldy once tasks have real content, and every edit is a
merge-conflict magnet. Separate files keep tasks self-describing, diffable and
independently movable, while the folders give an unambiguous board.

## Development

```bash
(cd crates/amd-ui-web && trunk build --release)   # needed before anything builds amd-ui
cargo test                    # unit tests (slugify, frontmatter, templates, the board)
bash tests/test.sh            # end-to-end CLI spec against throwaway git repos
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy -p amd-ui-web -p amd-ui-model --target wasm32-unknown-unknown -- -D warnings
cargo fmt --all --check
```

To work on the board with live reload: `cargo tauri dev -- --no-default-features`
in `crates/amd-ui` (it runs `trunk serve` for you; `--no-default-features`
serves the pages live instead of the ones built into the binary).

For screenshots, `scripts/demo-board.sh [DIR]` builds a demo project: a
fictional shop with every column, epics, a running sprint, assignees,
priorities, points, tags, a draft and locks, all made-up names at
example.com. It keeps its own config in `DIR/config`, so your own boards and
settings are untouched, and prints the command to open it.

`amd` was pure bash through v3; v4 is the same tool in Rust, which is what
buys the templating. The CLI, the board layout and the file format are
unchanged — v3 boards work as-is.

## Contributing

Bugs and ideas go in [issues](https://github.com/agile-md/amd/issues/new/choose);
they're pulled into the maintainers' board automatically and triaged there.
Pull requests are welcome: [CONTRIBUTING.md](CONTRIBUTING.md) has the build,
the checks CI runs, the branch and commit names, and how to become a
maintainer. Everyone taking part follows the
[code of conduct](CODE_OF_CONDUCT.md). Report a security problem privately,
through the repository's Security tab, not in an issue.

## Licence

MIT: see [LICENSE](LICENSE). Contributions are accepted under the same
licence.
