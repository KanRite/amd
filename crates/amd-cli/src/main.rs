//! amd — a tiny filesystem Kanban in markdown (agile-md).
//!
//! Tasks are markdown files that move between `todo/`, `doing/` and `done/` in
//! `<repo-root>/tasks`. Status is the folder; `git mv` is the audit trail; the
//! ticket format is a MiniJinja template you can edit.

mod completions;
mod form;
mod init;
mod render;
mod value;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::{Context, Result, bail};
use clap::builder::PossibleValuesParser;
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

use agile_md::board::{ABANDON, Board, Column};
use agile_md::create::{Draft, NewTicket};
use agile_md::registry::Registry;
use agile_md::templates::{DEFAULT_TEMPLATE, TEMPLATE_SUFFIX, Templates};
use agile_md::{branch, git, task, templates};

const AFTER_HELP: &str = "\
A <REF> is a task id (e.g. 7 or 007) or a unique slug substring. Leave it out
of `amd new` in a terminal and amd fills in a form; leave it out of a
destination verb (see below) and amd lists that destination instead of
moving anything.

One kind of ticket. A branch type — feature, bugfix, hotfix, release,
chore — is what gives it a branch, so `amd todo` on a bugfix titled
\"Crash on save\" creates and switches to bugfix/crash-on-save when it enters
todo/. Leave the type out and there is nothing to check out: the ticket
still moves, the working tree is left alone.

New tickets land in backlog/. `amd set` sizes and files them; `amd group`
divides the backlog into epics and sprints.

Tickets nest: `--parent <ref>` puts one under another, and the board shows
the tree. Nesting is the parent/child relationship; an epic or sprint is a
folder the backlog is divided into. A ticket can have both.

`amd todo`/`doing`/`done`/`backlog`/`abandon <ref>` move a ticket there;
leave off <ref> and it lists that destination instead of moving anything.
The same works for any epic by name: `amd checkout 7` files ticket 7 under
the checkout epic, `amd checkout` lists what's in it. Entering `todo` is the
one with a side effect: it creates or switches to the ticket's branch, the
same way `amd start` used to. Abandoning a ticket deletes that branch too,
if it's still empty — nothing lost, nothing left behind.

`amd init <project>` creates a private `<project>-tickets` GitHub repository
(under the GitHub organisation or personal account named <project> — pass
--org to use a different owner), clones it as a child of the current
directory, scaffolds the board inside it, and registers it. Must be run
from a plain directory, never from inside an existing git repository —
`amd init <project> --local-only` scaffolds the board in the repository
you're standing in instead, with no network and nothing created remotely,
the project name still recorded, just without a dedicated tickets repo.
<project> cannot be changed once set — every ticket records which
repository it's about using this name, and changing it later means finding
and updating every ticket by hand, on every clone, with no way to do it
atomically. Choose the name you'd want a stranger to see in a year.

Environment:
  AMD_DIR           board directory name under the repository root (default: tasks)
  AMD_BRANCH_TYPES  comma-separated branch types (default: feature, bugfix,
                    hotfix, release, chore)
  AMD_YES           set to 1 to create a missing board without prompting
  AMD_NO_INPUT      set to 1 to never prompt (same as --no-input)
  AMD_NO_BRANCH     set to 1 to never create branches (same as --no-branch)
  AMD_NO_REGISTER   set to 1 to keep the repository list manual
  NO_COLOR          set to any value for plain output (same as --plain)
  EDITOR            editor used by `amd edit` (default: vi)

AMD_BRANCH_TYPES, EDITOR and AMD_NO_REGISTER can also be set in the settings
file (`branch-types`, `editor`, `no-register`) — the environment variable
wins when both are set.

Creating a task interactively ends in $EDITOR with the rendered ticket, so
the notes and checklist get filled in there and then (--no-edit to skip).

Tasks are rendered from MiniJinja templates. `amd templates` lists them and
`amd templates eject task` writes an editable copy into <board>/templates/.
A template that uses `extra.<name>` gets asked for it in the form.";

#[derive(Parser, Debug)]
#[command(
    name = "amd",
    version,
    about = "A tiny filesystem Kanban: markdown tasks in todo/, doing/ and done/",
    after_help = AFTER_HELP,
    disable_help_subcommand = false
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    /// Never prompt; missing values are an error instead
    #[arg(long, global = true)]
    no_input: bool,
    /// Plain text output instead of the rich board
    #[arg(long, global = true)]
    plain: bool,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create or connect this project's dedicated tickets repository
    Init {
        /// The project's declared identity — cannot be changed once set
        #[arg(value_name = "PROJECT")]
        project: String,
        /// Scaffold the board in this repository instead of a dedicated
        /// tickets repository — no network, nothing created remotely
        #[arg(long)]
        local_only: bool,
        /// GitHub org/user to create the tickets repository under
        /// (default: the project name itself)
        #[arg(long, value_name = "ORG")]
        org: Option<String>,
    },
    /// Create a task in todo/ from a template
    New(NewArgs),
    /// Show all columns (the default command)
    Board,
    /// List a column: backlog, todo, doing, done or all
    #[command(alias = "list")]
    Ls {
        #[arg(
            default_value = "all",
            value_name = "COLUMN",
            value_parser = PossibleValuesParser::new(["backlog", "todo", "doing", "done", "all"]),
        )]
        column: String,
    },
    /// Move a task into backlog/, or list it with no ref
    Backlog {
        #[arg(value_name = "REF")]
        task: Option<String>,
    },
    /// Move a task into todo/, or list it with no ref — entering todo/
    /// creates or switches to the ticket's branch
    Todo {
        #[arg(value_name = "REF")]
        task: Option<String>,
        /// Branch to create instead of the one in the ticket
        #[arg(short, long, value_name = "NAME")]
        branch: Option<String>,
        /// Move the task without touching branches
        #[arg(long)]
        no_branch: bool,
    },
    /// Move a task into doing/, or list it with no ref
    Doing {
        #[arg(value_name = "REF")]
        task: Option<String>,
    },
    /// Move a task into done/, or list it with no ref
    Done {
        #[arg(value_name = "REF")]
        task: Option<String>,
    },
    /// Take a ticket off the board into archive/, or list the archive with
    /// no ref
    Abandon {
        #[arg(value_name = "REF")]
        task: Option<String>,
    },
    /// Print a task
    Show {
        #[arg(value_name = "REF")]
        task: Option<String>,
    },
    /// Open a task in $EDITOR
    Edit {
        #[arg(value_name = "REF")]
        task: Option<String>,
    },
    /// Assign a ticket, or clear it with no name
    Assign {
        #[arg(value_name = "REF")]
        task: String,
        /// Who it belongs to; `@me` is your git user name, empty unassigns
        #[arg(value_name = "WHO")]
        who: Option<String>,
    },
    /// Set a field on a ticket: points, epic, order or title
    Set {
        #[arg(value_name = "REF")]
        task: String,
        #[arg(
            value_name = "FIELD",
            value_parser = PossibleValuesParser::new(["points", "epic", "order", "title"]),
        )]
        field: String,
        /// The new value; empty clears the field
        #[arg(value_name = "VALUE", default_value = "")]
        value: String,
    },
    /// Epics and sprints: the folders the backlog is divided into
    Group {
        #[command(subcommand)]
        command: Option<GroupCmd>,
    },
    /// Register repositories so their boards can be seen together
    Repos {
        #[command(subcommand)]
        command: Option<ReposCmd>,
    },
    /// Link tickets together: both ends get the other in `related`
    Link {
        #[arg(value_name = "REF")]
        task: String,
        #[arg(value_name = "REF", required = true)]
        to: Vec<String>,
        /// Only record the link on the first ticket
        #[arg(long)]
        one_way: bool,
    },
    /// Open the desktop board across every registered repository (same window
    /// as the `amdui` command)
    Gui,
    /// Print a shell completion script (bash, zsh, fish, …)
    Completions {
        /// Shell to generate for; defaults to $SHELL
        #[arg(value_name = "SHELL")]
        shell: Option<Shell>,
    },
    /// Inspect and customise the task templates
    Templates {
        #[command(subcommand)]
        command: Option<TemplateCmd>,
    },
    /// Move a task into an epic, or list it with no ref — checked against
    /// the board's current epics (the four columns and `abandon` are their
    /// own named commands above, and always win if a name collides)
    #[command(external_subcommand)]
    Verb(Vec<String>),
}

#[derive(Args, Debug)]
struct NewArgs {
    /// Task title; omit it in a terminal to fill in the form
    #[arg(value_name = "TITLE")]
    title: Option<String>,
    /// Branch type: feature, bugfix, hotfix, release, chore. Leave it out for
    /// a ticket with no branch
    #[arg(
        long = "branch-type",
        alias = "type",
        value_name = "TYPE",
        value_parser = value::BranchType,
    )]
    branch_type: Option<String>,
    /// The ticket this one sits under (an epic, a story, anything)
    #[arg(short = 'p', long, value_name = "REF")]
    parent: Option<String>,
    /// Who it's assigned to (nobody by default; `@me` is you). Tickets can be
    /// created now and assigned later
    #[arg(short = 'a', long, alias = "owner", value_name = "WHO")]
    assignee: Option<String>,
    /// Another ticket this one depends on or relates to (repeatable)
    #[arg(short = 'r', long = "related", value_name = "REF")]
    related: Vec<String>,
    /// Tag to add (repeatable)
    #[arg(short = 't', long = "tag", value_name = "TAG")]
    tags: Vec<String>,
    /// Template to render the ticket from (default: ticket). A board can add
    /// its own under <board>/templates/
    #[arg(
        short = 'T',
        long = "template",
        alias = "ticket",
        default_value = DEFAULT_TEMPLATE,
        value_name = "TYPE",
        value_parser = value::TicketType,
    )]
    template: String,
    /// Ask for every field, even the ones given as arguments
    #[arg(short, long)]
    interactive: bool,
    /// Extra template variable, available as extra.KEY (repeatable)
    #[arg(short = 's', long = "set", value_name = "KEY=VALUE", value_parser = parse_key_value)]
    set: Vec<(String, String)>,
    /// Open the ticket body in $EDITOR, even when the title came from the CLI
    #[arg(short, long)]
    edit: bool,
    /// Create the ticket from the template without opening an editor
    #[arg(long, conflicts_with = "edit")]
    no_edit: bool,
}

#[derive(Subcommand, Debug)]
enum GroupCmd {
    /// List the epics and sprints in this backlog
    List,
    /// Create an epic
    Epic {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long, value_name = "TEXT", default_value = "")]
        description: String,
    },
    /// Create a sprint
    Sprint {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long, value_name = "TEXT", default_value = "")]
        description: String,
        /// How long it runs
        #[arg(long, value_name = "DAYS", default_value_t = agile_md::group::DEFAULT_DAYS)]
        days: u32,
    },
    /// Start a sprint. There is no way back: a started sprint takes nothing
    /// more and gives nothing back
    Start {
        #[arg(value_name = "NAME")]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum ReposCmd {
    /// List the registered repositories (the default)
    List,
    /// Register a repository; defaults to the one you're in
    Add {
        #[arg(value_name = "PATH")]
        path: Option<PathBuf>,
    },
    /// Unregister a repository, by path or by name
    Remove {
        #[arg(value_name = "PATH|NAME")]
        repo: String,
    },
}

#[derive(Subcommand, Debug)]
enum TemplateCmd {
    /// List the available templates (the default)
    List,
    /// Print a template's source
    Show {
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Copy a built-in template into <board>/templates/ so you can edit it
    Eject {
        #[arg(default_value = DEFAULT_TEMPLATE, value_name = "NAME")]
        name: String,
        /// Overwrite an existing board template
        #[arg(short, long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    // Behave like a normal Unix tool when the reader goes away — `amd board |
    // head` is an ordinary thing to do. Rust ignores SIGPIPE by default, which
    // turns a closed pipe into a panic on the next print rather than a quiet
    // exit.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("amd: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    form::set_no_input(cli.no_input);
    render::set_plain(cli.plain);
    let command = cli.command.unwrap_or(Cmd::Board);

    // Completions don't need a board — and often run before there is one.
    if let Cmd::Completions { shell } = command {
        return completions::print(shell, &mut Cli::command());
    }

    // Nor does the registry: it's a list of repositories, and asking what's on
    // it shouldn't put the current one there. `amd repos` works anywhere.
    if let Cmd::Repos { command } = command {
        return cmd_repos(command.unwrap_or(ReposCmd::List));
    }

    // The desktop board draws every registered repository, so it must not
    // insist on standing in one — `amd gui` works from anywhere. The `amdui`
    // binary is the same call without the subcommand.
    if let Cmd::Gui = command {
        return cmd_gui();
    }

    // `init` is the one command that runs without an existing board.
    if let Cmd::Init {
        project,
        local_only,
        org,
    } = command
    {
        return init::run(project, local_only, org);
    }

    let board = ensure_board()?;
    // Every repository you work in joins the list, so the boards you actually
    // use are all there next session without anyone curating them.
    agile_md::registry::remember(&board);
    // Notices if this board's declared project identity has silently
    // changed since amd last saw it — see project_trust.rs.
    agile_md::project_trust::check(&board)?;
    match command {
        Cmd::Init { .. } | Cmd::Completions { .. } | Cmd::Repos { .. } | Cmd::Gui => {
            unreachable!("handled above")
        }
        Cmd::New(args) => cmd_new(&board, args),
        Cmd::Board => cmd_ls(&board, "all"),
        Cmd::Ls { column } => cmd_ls(&board, &column),
        Cmd::Backlog { task } => {
            cmd_destination(&board, Destination::Column(Column::Backlog), task)
        }
        Cmd::Todo {
            task,
            branch,
            no_branch,
        } => match task {
            Some(reference) => {
                let task = board.find(&reference)?;
                cmd_enter_todo(&board, &task, branch, no_branch)
            }
            None => render::columns(&board, &[Column::Todo]),
        },
        Cmd::Doing { task } => cmd_destination(&board, Destination::Column(Column::Doing), task),
        Cmd::Done { task } => cmd_destination(&board, Destination::Column(Column::Done), task),
        Cmd::Abandon { task } => cmd_destination(&board, Destination::Abandon, task),
        Cmd::Verb(args) => {
            let mut args = args.into_iter();
            let name = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("usage: amd <destination> [ref]"))?;
            let task = args.next();
            if let Some(extra) = args.next() {
                bail!("amd {name}: unexpected argument '{extra}'");
            }
            let destination = resolve_destination(&board, &name)?;
            cmd_destination(&board, destination, task)
        }
        Cmd::Show { task } => {
            let task = resolve(&board, task, "show", &Column::ALL)?;
            let body =
                fs::read(&task.path).with_context(|| format!("reading {}", task.path.display()))?;
            io::stdout().write_all(&body)?;
            Ok(())
        }
        Cmd::Edit { task } => {
            let task = resolve(&board, task, "edit", &Column::ALL)?;
            open_editor(&task.path)
        }
        Cmd::Set { task, field, value } => {
            let task = resolve(&board, Some(task), "set", &Column::ALL)?;
            match field.as_str() {
                "points" => {
                    task.set_points(task.hash()?, &value)?;
                    println!("{} sized {}", task.id_display(), display(&value));
                }
                // Filing goes through the board, not the ticket: it moves the
                // file, and the sprint rules live there.
                "epic" => {
                    let epic = (!value.is_empty()).then_some(value.as_str());
                    board.set_epic(&task, epic)?;
                    println!("{} filed under {}", task.id_display(), display(&value));
                }
                "order" => {
                    let rank: f64 = value
                        .parse()
                        .with_context(|| format!("{value} is not a number"))?;
                    task.set_order(task.hash()?, rank)?;
                    println!("{} ranked {rank}", task.id_display());
                }
                "title" => {
                    task.set_title(task.hash()?, &value)?;
                    println!("{} retitled", task.id_display());
                }
                other => bail!("unknown field {other}"),
            }
            Ok(())
        }
        Cmd::Group { command } => cmd_group(&board, command.unwrap_or(GroupCmd::List)),
        Cmd::Assign { task, who } => {
            let task = resolve(&board, Some(task), "assign", &Column::ALL)?;
            let who = resolve_assignee(who.as_deref());
            task.assign(task.hash()?, &who)?;
            match who.is_empty() {
                true => println!("{} unassigned", task.id_display()),
                false => println!("{} assigned to {who}", task.id_display()),
            }
            Ok(())
        }
        Cmd::Link { task, to, one_way } => cmd_link(&board, &task, &to, one_way),
        Cmd::Templates { command } => cmd_templates(&board, command.unwrap_or(TemplateCmd::List)),
    }
}

/// Move a task into todo/ and put the working tree on its branch. This is
/// what `amd start` used to do on entry to `doing/`; the trigger moved
/// earlier, to `todo/`-entry, because "todo/ is what you have committed to"
/// — the workspace should be ready once you've committed to the work, not
/// only once you start typing (docs/adr/0006).
///
/// Order matters: the `git mv` is staged first so it travels with the switch
/// and lands on the task's own branch.
fn cmd_enter_todo(
    board: &Board,
    task: &task::Task,
    override_branch: Option<String>,
    no_branch: bool,
) -> Result<()> {
    // Read everything off the ticket before the move — afterwards the path is
    // stale. A ticket with no branch type has nothing to check out, so there's
    // nothing to switch to unless --branch says otherwise.
    let wanted = match override_branch {
        Some(name) => Some(name),
        None => match task.branch() {
            Some(name) => Some(name),
            // A ticket edited by hand may name a branch type but no branch.
            None => match task.branch_type() {
                Some(kind) => {
                    let name = branch::for_title(&kind, &task.title())?;
                    (!name.is_empty()).then_some(name)
                }
                None => None,
            },
        },
    };

    move_and_report(board, task, Column::Todo)?;

    if no_branch || std::env::var("AMD_NO_BRANCH").as_deref() == Ok("1") {
        return Ok(());
    }
    let Some(name) = wanted else {
        eprintln!("amd: no branch type on this ticket; left the branch alone");
        return Ok(());
    };
    branch::validate(&name)?;
    if git::current_branch(&board.root).as_deref() == Some(name.as_str()) {
        println!("already on {name}");
        return Ok(());
    }
    let exists = git::branch_exists(&board.root, &name);
    git::switch_branch(&board.root, &name, !exists)?;
    println!("{} {name}", if exists { "switched to" } else { "branch" });
    Ok(())
}

/// Link tickets together. The relation is symmetric by default — "related"
/// reads the same from either end — so both files are updated unless the
/// caller asks for one direction only.
fn cmd_link(board: &Board, reference: &str, to: &[String], one_way: bool) -> Result<()> {
    let task = board.find(reference)?;
    let id = task.id_display();
    let mut others = Vec::new();
    for other in to {
        let other = board.find(other)?;
        if other.path == task.path {
            bail!("a task can't be related to itself");
        }
        others.push(other);
    }

    let ids: Vec<String> = others.iter().map(|other| other.id_display()).collect();
    if task.add_related(task.hash()?, &ids)? {
        println!("{} related: {}", id, ids.join(", "));
    } else {
        println!("{id} was already related to {}", ids.join(", "));
    }
    if one_way {
        return Ok(());
    }
    for other in &others {
        if other.add_related(other.hash()?, std::slice::from_ref(&id))? {
            println!("{} related: {id}", other.id_display());
        }
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn cmd_gui() -> Result<()> {
    agile_md::gui::run()
}

#[cfg(not(feature = "gui"))]
fn cmd_gui() -> Result<()> {
    bail!("this build has no desktop board — rebuild with --features gui")
}

/// Move a ticket and say so. The wording is the CLI's, not the library's —
/// `Board::move_task` returns where the ticket landed and leaves the reporting
/// to whoever asked for the move.
fn move_and_report(board: &Board, task: &task::Task, to: Column) -> Result<()> {
    board.move_task(task, to)?;
    println!("moved {} -> {to}/", task.file_name());
    Ok(())
}

/// Where a destination verb can send a ticket: one of the four columns,
/// taking it off the board entirely, or filing it under an existing epic.
enum Destination {
    Column(Column),
    Abandon,
    Epic(String),
}

/// Check a destination verb against the board — a column, `abandon`, or the
/// name of an epic that already exists — and say exactly what's valid when
/// it's none of those, since that list is the whole point of the error.
fn resolve_destination(board: &Board, name: &str) -> Result<Destination> {
    if let Some(column) = Column::parse(name) {
        return Ok(Destination::Column(column));
    }
    if name.eq_ignore_ascii_case(ABANDON) {
        return Ok(Destination::Abandon);
    }
    let epics = board.epics_in(Column::Backlog)?;
    if epics.iter().any(|epic| epic == name) {
        return Ok(Destination::Epic(name.to_string()));
    }
    let columns: Vec<String> = Column::ALL.iter().map(|c| c.to_string()).collect();
    let epics_display = if epics.is_empty() {
        "none".to_string()
    } else {
        epics.join(", ")
    };
    bail!(
        "'{name}' is not a column, '{ABANDON}', or an epic on this board\n      (columns: {}; epics: {epics_display})",
        columns.join(", "),
    );
}

/// Move a ticket to a destination, or — with no ref — list what's there.
/// Every destination verb goes through this one function, static columns and
/// dynamic epic names alike, so they can't drift from each other.
fn cmd_destination(board: &Board, destination: Destination, task: Option<String>) -> Result<()> {
    match (destination, task) {
        (Destination::Column(column), Some(reference)) => {
            let task = board.find(&reference)?;
            move_and_report(board, &task, column)
        }
        (Destination::Column(column), None) => render::columns(board, &[column]),
        (Destination::Abandon, Some(reference)) => {
            let task = board.find(&reference)?;
            let name = task.file_name();
            let (_, deleted_branch) = board.archive(&task)?;
            println!("archived {name} -> archive/");
            if let Some(branch) = deleted_branch {
                println!("deleted {branch} (empty)");
            }
            Ok(())
        }
        (Destination::Abandon, None) => render::named(ABANDON, board.archived()?),
        (Destination::Epic(name), Some(reference)) => {
            let task = board.find(&reference)?;
            board.set_epic(&task, Some(&name))?;
            println!("{} filed under {name}", task.id_display());
            Ok(())
        }
        (Destination::Epic(name), None) => {
            let tasks = board.tasks_in_group(&name)?;
            render::named(&name, tasks)
        }
    }
}

/// An empty value reads better as a word than as nothing at all.
fn display(value: &str) -> String {
    match value.is_empty() {
        true => "(cleared)".to_string(),
        false => value.to_string(),
    }
}

fn cmd_group(board: &Board, command: GroupCmd) -> Result<()> {
    use agile_md::group::{Group, Kind, State};
    match command {
        GroupCmd::List => {
            let groups = board.groups()?;
            if groups.is_empty() {
                println!("(no epics or sprints — add one with: amd group epic <name>)");
                return Ok(());
            }
            for group in groups {
                let tickets = board.tasks_in_group(&group.name)?;
                let points: i64 = tickets
                    .iter()
                    .filter_map(|task| task.points())
                    .filter_map(|p| p.trim().parse::<i64>().ok())
                    .sum();
                let detail = match group.is_sprint() {
                    true => format!("sprint  {}d  {}", group.days, group.state.as_str()),
                    false => "epic".to_string(),
                };
                println!(
                    "{:<24} {detail:<22} {} ticket(s), {points} point(s)",
                    group.name,
                    tickets.len()
                );
            }
            Ok(())
        }
        GroupCmd::Epic { name, description } => {
            let group = Group {
                dir: board.dir(Column::Backlog).join(&name),
                name: name.clone(),
                kind: Kind::Epic,
                description,
                days: agile_md::group::DEFAULT_DAYS,
                state: State::Pending,
            };
            board.create_group(&group)?;
            println!("created epic {name}");
            Ok(())
        }
        GroupCmd::Sprint {
            name,
            description,
            days,
        } => {
            let group = Group {
                dir: board.dir(Column::Backlog).join(&name),
                name: name.clone(),
                kind: Kind::Sprint,
                description,
                days,
                state: State::Pending,
            };
            board.create_group(&group)?;
            println!("created sprint {name} ({days} days)");
            Ok(())
        }
        GroupCmd::Start { name } => {
            let mut group = board.group(&name)?;
            group.start()?;
            println!("{name} started — its tickets are now fixed");
            Ok(())
        }
    }
}

/// The registry is a list of repositories, not a copy of their tickets: the
/// markdown is the source of truth and it changes under you.
fn cmd_repos(command: ReposCmd) -> Result<()> {
    let mut registry = Registry::load();
    match command {
        ReposCmd::List => {
            if registry.entries.is_empty() {
                println!("(no repositories registered — add one with: amd repos add)");
                return Ok(());
            }
            let width = registry
                .entries
                .iter()
                .map(|entry| entry.name.len())
                .max()
                .unwrap_or(0);
            for entry in &registry.entries {
                let board = if entry.has_board() {
                    ""
                } else {
                    "  (no board)"
                };
                println!("{:width$}  {}{board}", entry.name, entry.root.display());
            }
            Ok(())
        }
        ReposCmd::Add { path } => {
            let path = match path {
                Some(path) => path,
                None => git::repo_root()
                    .ok_or_else(|| anyhow::anyhow!("not inside a git repository"))?,
            };
            let added = registry.add(&path)?;
            registry.save()?;
            match added {
                true => println!("registered {}", path.display()),
                false => println!("{} was already registered", path.display()),
            }
            Ok(())
        }
        ReposCmd::Remove { repo } => {
            if !registry.remove(&repo) {
                bail!("'{repo}' is not registered");
            }
            registry.save()?;
            println!("unregistered {repo}");
            Ok(())
        }
    }
}

/// `@me` is whoever git says you are — the name that ends up in the commits.
fn resolve_assignee(who: Option<&str>) -> String {
    match who {
        Some("@me") => git::config("user.name").unwrap_or_else(|| "me".to_string()),
        Some(who) => who.trim().to_string(),
        None => String::new(),
    }
}

/// Open the board, offering to create one when it isn't there: prompt when
/// interactive, create outright with `AMD_YES=1`, and otherwise fail with a
/// clear message — never block waiting on a pipe.
fn ensure_board() -> Result<Board> {
    let board = Board::locate()?;
    if board.root.is_dir() {
        return Ok(board);
    }
    let create = if std::env::var("AMD_YES").as_deref() == Ok("1") {
        true
    } else if form::available() {
        eprintln!("No task board found at {}", board.root.display());
        form::confirm("Create an empty board here?")?
    } else {
        return Board::open();
    };
    if !create {
        bail!("no board created");
    }
    board.create()?;
    eprintln!("Created board at {}", board.root.display());
    Ok(board)
}

/// Resolve a task reference, or offer a picker when it was left out.
fn resolve(
    board: &Board,
    reference: Option<String>,
    command: &str,
    columns: &[Column],
) -> Result<task::Task> {
    if let Some(reference) = reference {
        return board.find(&reference);
    }
    if !form::available() {
        bail!("usage: amd {command} <ref>");
    }
    let mut choices = Vec::new();
    for column in columns {
        choices.extend(board.tasks_in(*column)?);
    }
    if choices.is_empty() {
        let names: Vec<String> = columns.iter().map(|c| format!("{c}/")).collect();
        bail!("no tasks in {}", names.join(" or "));
    }
    form::select_task(&format!("Which task should amd {command}?"), choices)
}

fn cmd_new(board: &Board, args: NewArgs) -> Result<()> {
    let templates = Templates::load(board)?;
    // A bare `amd new` (or -i) fills the whole form in; otherwise only the
    // fields the chosen template needs and the arguments didn't supply.
    let full_form = args.interactive || args.title.is_none();
    if full_form && !form::available() {
        bail!("usage: amd new \"<title>\" [-t tag ...]");
    }

    // The title first: it's the one thing every ticket has, and it names the
    // file whatever the labels turn out to be.
    let title = match args.title {
        Some(title) if !args.interactive => title,
        title => form::title(title.as_deref())?,
    };
    branch::validate_sluggable(&title)?;

    // A branch type is optional, and empty by default: a ticket only gets a
    // branch when you say what kind of work it is.
    let branch_type = match args.branch_type {
        Some(chosen) => branch::normalise(&chosen),
        None if full_form => branch::normalise(&form::select(
            "Branch type:",
            branch::choices(),
            &branch::NONE.to_string(),
        )?),
        None => String::new(),
    };
    let template = args.template.clone();

    // One parent instead of epic and story: nesting gives both, and any depth
    // past them.
    let parent = match args.parent {
        Some(parent) => Some(parent),
        None if full_form => {
            let answer = form::label(
                "Parent:",
                "",
                board.stems()?,
                "the ticket this one sits under; blank for none",
            )?;
            (!answer.is_empty()).then_some(answer)
        }
        None => None,
    };

    let assignee = match args.assignee {
        Some(who) => resolve_assignee(Some(&who)),
        None if full_form => resolve_assignee(Some(&form::label(
            "Assignee:",
            "",
            board.assignees()?,
            "who's doing it; blank for nobody, @me for you",
        )?)),
        None => String::new(),
    };

    let related = if full_form && args.related.is_empty() {
        form::related("", board.stems()?)?
    } else {
        args.related
    };

    let tags = if full_form {
        form::tags(&args.tags.join(", "), board.tags()?)?
    } else {
        args.tags
    };

    // Fields the template itself asks for: `{{ extra.owner }}` becomes a
    // question, so a template can't reference a value nobody collected.
    let mut extra: BTreeMap<String, String> = args.set.into_iter().collect();
    let required = templates.required_extras(&template)?;
    let missing: Vec<&String> = required
        .iter()
        .filter(|key| !extra.contains_key(*key))
        .collect();
    if !missing.is_empty() {
        if !form::available() {
            let flags: Vec<String> = missing.iter().map(|key| format!("--set {key}=…")).collect();
            bail!("template '{template}' needs {}", flags.join(" "));
        }
        for key in missing {
            let answer = form::text(&format!("{}:", templates::field_label(key)), None, None)?;
            extra.insert(key.clone(), answer);
        }
    }

    let draft = Draft::prepare(
        board,
        &templates,
        NewTicket {
            title,
            assignee,
            template,
            branch_type,
            parent,
            related,
            tags,
            extra,
        },
    )?;

    // The form doesn't stop at the metadata: unless told otherwise, it opens
    // the rendered ticket so the notes and checklist are filled in as part of
    // creating it. Nothing is written until the editor exits, so abandoning
    // the edit leaves no half-made task behind.
    let edit = !args.no_edit && form::available() && (args.edit || full_form);
    let body = if edit {
        Some(form::body(&draft.body, OsStr::new(&editor_command()))?)
    } else {
        None
    };

    let note = if draft.branch.is_empty() {
        String::new()
    } else {
        format!(" ({})", draft.branch)
    };
    let name = draft
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let created = draft.write(board, body)?;
    for id in &created.linked {
        println!("{id} now links to {}", created.task.id_display());
    }
    println!("created {}/{name}{note}", Column::Backlog);
    Ok(())
}

fn cmd_ls(board: &Board, column: &str) -> Result<()> {
    if column.eq_ignore_ascii_case("all") {
        return render::columns(board, &Column::ALL);
    }
    let column = Column::parse(column)
        .with_context(|| format!("unknown column '{column}' (todo, doing, done or all)"))?;
    render::columns(board, &[column])
}

fn cmd_templates(board: &Board, command: TemplateCmd) -> Result<()> {
    let templates = Templates::load(board)?;
    match command {
        TemplateCmd::List => {
            let width = templates
                .list()
                .map(|(name, _)| name.len())
                .max()
                .unwrap_or(0);
            for (name, source) in templates.list() {
                println!("{name:width$}  {}", source.describe());
            }
            Ok(())
        }
        TemplateCmd::Show { name } => {
            print!("{}", templates.source_text(&name)?);
            Ok(())
        }
        TemplateCmd::Eject { name, force } => {
            let source = templates::builtin_source(&name)
                .with_context(|| format!("'{name}' is not a built-in template"))?;
            let dir = board.templates_dir();
            fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
            let path = dir.join(format!("{name}{TEMPLATE_SUFFIX}"));
            if path.exists() && !force {
                bail!(
                    "{} already exists (use --force to overwrite)",
                    path.display()
                );
            }
            fs::write(&path, source).with_context(|| format!("writing {}", path.display()))?;
            println!("wrote {}", path.display());
            Ok(())
        }
    }
}

/// `$EDITOR`, else the settings file's `editor` key, else `vi` — used by
/// both `amd edit` and the form's body step. GUI-only, so this lives in the
/// CLI rather than amd-lib: the GUI never spawns an editor.
fn editor_command() -> String {
    let config = agile_md::settings::Config::load();
    agile_md::settings::resolved("EDITOR", &config, "editor")
        .filter(|editor| !editor.is_empty())
        .unwrap_or_else(|| "vi".to_string())
}

fn open_editor(path: &Path) -> Result<()> {
    let editor = editor_command();
    let status = Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("running {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    Ok(())
}

fn parse_key_value(raw: &str) -> Result<(String, String), String> {
    match raw.split_once('=') {
        Some((key, value)) if !key.is_empty() => Ok((key.to_string(), value.to_string())),
        _ => Err(format!("expected KEY=VALUE, got '{raw}'")),
    }
}
