#!/usr/bin/env bash
#
# End-to-end spec for the amd CLI. Run: bash tests/test.sh
#
# Builds the binary with cargo, then drives it against throwaway git repos.
# Unit tests for the pure logic (slugify, frontmatter, templates) live in the
# Rust sources and run under `cargo test`.
#
# AMD_NO_INPUT=1 keeps the run hermetic: the suite covers the non-interactive
# paths, and without it a prompt would block when the suite is run from a
# terminal.
#
set -uo pipefail

export AMD_NO_INPUT=1
# Keep the suite away from the real registry and theme: `amd register` writes
# to the config directory, and a test must never touch the user's.
XDG_CONFIG_HOME="$(mktemp -d)/config"
export XDG_CONFIG_HOME

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ -n "${AMD_BIN:-}" ]; then
  AMD="${AMD_BIN}"
else
  echo "building amd..."
  (cd "${ROOT}" && cargo build --quiet) || exit 1
  AMD="${ROOT}/target/debug/amd"
fi
[ -x "${AMD}" ] || { echo "no amd binary at ${AMD}"; exit 1; }

pass=0
fail=0
assert() { # description  command...
  local d="$1"; shift
  if "$@" >/dev/null 2>&1; then echo "  ok   - ${d}"; pass=$((pass + 1))
  else echo "  FAIL - ${d}"; fail=$((fail + 1)); fi
}

tmp="$(mktemp -d)"
nongit="$(mktemp -d)"
trap 'rm -rf "${tmp}" "${nongit}"' EXIT
cd "${tmp}" || exit 1
git init -q; git config user.email t@t.co; git config user.name t

echo "init:"
"${AMD}" init acme --local-only >/dev/null
assert "init creates backlog/todo/doing/done" \
  test -d tasks/backlog -a -d tasks/todo -a -d tasks/doing -a -d tasks/done
assert "init writes the repository README, not inside tasks/" \
  bash -c "test -f README.md -a ! -f tasks/README.md"
assert "init records the declared project in the repository, not inside tasks/" \
  bash -c "grep -q '^project = acme\$' .settings && test ! -f tasks/.settings"
assert "init records whoever ran it as the project's first user (ticket 090)" \
  grep -qx 'users = t <t@t.co>' .settings
assert "a second init with the same project is a silent no-op" \
  "${AMD}" init acme --local-only
assert "init with a different project is refused, naming both" \
  bash -c "'${AMD}' init other --local-only 2>&1 | grep -q \"already 'acme'\" && grep -q '^project = acme\$' .settings"
assert "an invalid project name is rejected before anything is created" \
  bash -c "'${AMD}' init 'not valid' --local-only 2>&1 | grep -q 'letters, numbers, hyphens, periods, colons and underscores'"

periods_and_underscores="$(mktemp -d)"
( cd "${periods_and_underscores}" && git init -q && git config user.email t@t.co && git config user.name t )
assert "periods and underscores are valid in a project name" \
  bash -c "cd '${periods_and_underscores}' && '${AMD}' init 'a.valid_name' --local-only >/dev/null && grep -q '^project = a.valid_name\$' .settings"
# init --local-only registers the board it creates locally — undo it here
# so it doesn't leave a stray local-registry entry ahead of the dedicated
# "registration:" tests below, which check that file's exact contents.
"${AMD}" deregister "$(basename "${periods_and_underscores}")" >/dev/null 2>&1

echo "init: a dedicated tickets repository, found locally:"
# Deliberately a plain, non-git directory: `amd init <project>` must not
# require being run from inside an existing git repository — it creates or
# finds `<project>-tickets`/`<project>-documents` as children of wherever
# it's run from. Both pre-created here (empty, local-only git repos, no
# 'origin') so `init` finds both and never calls out to `gh` at all —
# hitting the real network for a nonexistent "widgets" owner would just
# fail, and this test has nothing to do with `gh repo create` itself.
plain_dir="$(mktemp -d)"
( cd "${plain_dir}" && git init -q widgets-tickets \
  && cd widgets-tickets && git config user.email t@t.co && git config user.name t )
( cd "${plain_dir}" && git init -q widgets-documents \
  && cd widgets-documents && git config user.email t@t.co && git config user.name t )
assert "amd init <project> works from a plain, non-git directory" \
  bash -c "cd '${plain_dir}' && '${AMD}' init widgets >/dev/null && test -d '${plain_dir}/widgets-tickets/tasks/backlog'"
assert "the found-locally tickets repo's own board records the project" \
  bash -c "grep -q '^project = widgets\$' '${plain_dir}/widgets-tickets/.settings'"
assert "the found-locally documents repo is scaffolded, not a board" \
  bash -c "test -d '${plain_dir}/widgets-documents/adr' \
    && test ! -d '${plain_dir}/widgets-documents/tasks' \
    && grep -q '^widgets\$' '${plain_dir}/widgets-documents/.settings/project'"
assert "init records you as the first user of a found-locally project" \
  grep -qx 'users = t <t@t.co>' "${plain_dir}/widgets-tickets/.settings"
assert "...in a commit of its own, since the repository already existed" \
  bash -c "[ \"\$(git -C '${plain_dir}/widgets-tickets' log -1 --format=%s)\" = 'init: add t <t@t.co> to users' ]"
assert "running init again doesn't list you twice or commit again" \
  bash -c "cd '${plain_dir}' && before=\"\$(git -C widgets-tickets rev-parse HEAD)\" \
    && '${AMD}' init widgets >/dev/null 2>&1 \
    && [ \"\$(git -C widgets-tickets rev-parse HEAD)\" = \"\${before}\" ] \
    && [ \"\$(grep -c '^users = ' widgets-tickets/.settings)\" -eq 1 ]"
# Deregistering by alias both proves resolution actually works and cleans
# up the registry ahead of the dedicated "registration:"/"aliases:" tests
# below, which check that file's exact contents.
assert "amd init aliased the tickets repo 'tickets'" \
  bash -c "'${AMD}' deregister tickets | grep -q 'deregistered widgets-tickets'"
assert "amd init aliased the documents repo 'documents'" \
  bash -c "'${AMD}' deregister documents | grep -q 'deregistered widgets-documents'"

echo "init: an existing project made before ticket 090 gets its first user:"
# A project whose remote has no users yet, cloned locally the way its
# creator already has it; re-running init adds them and pushes.
backfill="$(mktemp -d)"
maker="GIT_CONFIG_GLOBAL=${backfill}/gitconfig GIT_CONFIG_NOSYSTEM=1 XDG_CONFIG_HOME=${backfill}/config"
printf '[user]\n\tname = maker\n\temail = maker@t.co\n[commit]\n\tgpgsign = false\n' >"${backfill}/gitconfig"
(
  set -e
  export GIT_CONFIG_GLOBAL="${backfill}/gitconfig" GIT_CONFIG_NOSYSTEM=1 XDG_CONFIG_HOME="${backfill}/config"
  cd "${backfill}" && git init -q -b main seed && cd seed
  "${AMD}" init old --local-only >/dev/null
  sed -i '/^users = /d' .settings
  git add -A && git commit -q -m "seed for ticket 090"
  git init -q --bare -b main "${backfill}/origin/old-tickets.git"
  git push -q "${backfill}/origin/old-tickets.git" HEAD:main
  mkdir -p "${backfill}/work" && cd "${backfill}/work"
  git clone -q "${backfill}/origin/old-tickets.git" old-tickets
  git init -q old-documents
)
assert "re-running init on it adds you and says so" \
  bash -c "cd '${backfill}/work' && out=\"\$(env ${maker} '${AMD}' init old 2>/dev/null)\" \
    && grep -q 'added you (maker <maker@t.co>)' <<<\"\${out}\""
assert "...pushed, so the remote lists you" \
  bash -c "git --git-dir='${backfill}/origin/old-tickets.git' show main:.settings | grep -qx 'users = maker <maker@t.co>'"
nobody="$(mktemp -d)"
( cd "${nobody}" && git init -q )
assert "init with no git email still succeeds, and warns that you weren't added" \
  bash -c "cd '${nobody}' && env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 XDG_CONFIG_HOME='${nobody}/config' \
    '${AMD}' init lonely --local-only 2>&1 >/dev/null | grep -q \"weren't added to lonely's users\" \
    && grep -qx 'project = lonely' .settings && ! grep -q '^users' .settings"
rm -rf "${backfill}" "${nobody}"

echo "init: refuses to run inside an existing git repository:"
guard_test="$(mktemp -d)"
( cd "${guard_test}" && git init -q code_repo \
  && cd code_repo && git config user.email t@t.co && git config user.name t \
  && git commit -q --allow-empty -m seed )
assert "amd init <project> refuses to run inside an existing repository" \
  bash -c "cd '${guard_test}/code_repo' && '${AMD}' init gadgets 2>&1 | grep -q 'must not be run inside a git repository'"
assert "nothing was created by the refused attempt" \
  test ! -e "${guard_test}/gadgets-tickets" -a ! -e "${guard_test}/code_repo/gadgets-tickets" \
  -a ! -e "${guard_test}/gadgets-documents" -a ! -e "${guard_test}/code_repo/gadgets-documents"

echo "join:"
# `amd join` clones over plain git, so local bare repositories stand in for
# GitHub: AMD_GIT_BASE points it at "${origin}/", and the URL it builds is
# "${origin}/<owner>/<project>-tickets.git". Every simulated person gets
# their own global git config (their identity) and their own config
# directory (their registry), the way two machines would — and so none of
# this touches the suite's own registry, which "registration:" below checks
# exactly.
origin="$(mktemp -d)"
people="$(mktemp -d)"
person() { # name email -> prints the env assignments for that person
  local home="${people}/$1"
  mkdir -p "${home}/config" "${home}/work"
  printf '[user]\n\tname = %s\n\temail = %s\n[commit]\n\tgpgsign = false\n' "$1" "$2" >"${home}/gitconfig"
  echo "GIT_CONFIG_GLOBAL=${home}/gitconfig XDG_CONFIG_HOME=${home}/config AMD_GIT_BASE=${origin}/"
}
seed_bare() { # project kind -> a bare repo on main, seeded from the current directory
  git init -q --bare -b main "${origin}/$1/$1-$2.git"
  git push -q "${origin}/$1/$1-$2.git" HEAD:main
}
(
  set -e
  export GIT_CONFIG_GLOBAL="${people}/seed.gitconfig" XDG_CONFIG_HOME="${people}/seed-config"
  printf '[user]\n\tname = seed\n\temail = seed@t.co\n' >"${GIT_CONFIG_GLOBAL}"
  seed="$(mktemp -d)"
  cd "${seed}" && git init -q -b main acme-tickets && cd acme-tickets
  "${AMD}" init acme --local-only >/dev/null
  git add -A && git commit -q -m seed && seed_bare acme tickets
  cd "${seed}" && git init -q -b main acme-documents && cd acme-documents
  git commit -q --allow-empty -m seed && seed_bare acme documents
  # A project whose tickets repository has no board in it at all.
  cd "${seed}" && git init -q -b main blank && cd blank
  git commit -q --allow-empty -m seed && seed_bare blank tickets && seed_bare blank documents
  rm -rf "${seed}"
)
remote_settings() { git --git-dir="${origin}/acme/acme-tickets.git" show main:.settings; }

ada="$(person ada ada@t.co)"
assert "amd join clones both repositories side by side" \
  bash -c "cd '${people}/ada/work' && env ${ada} '${AMD}' join acme >/dev/null \
    && test -d acme-tickets/tasks/backlog && test -d acme-documents/.git"
assert "the joining user is added to the remote's users" \
  bash -c "$(declare -f remote_settings); origin='${origin}'; remote_settings | grep -q '^users = .*ada <ada@t.co>'"
assert "both repositories are registered and aliased for the joining user" \
  bash -c "cd '${people}/ada/work' && env ${ada} '${AMD}' deregister tickets | grep -q 'deregistered acme-tickets' \
    && env ${ada} '${AMD}' deregister documents | grep -q 'deregistered acme-documents'"
assert "joining again (fresh directory) doesn't list the user twice" \
  bash -c "mkdir -p '${people}/ada/again' && cd '${people}/ada/again' \
    && env ${ada} '${AMD}' join acme | grep -q 'already one of its users' \
    && [ \"\$(git --git-dir='${origin}/acme/acme-tickets.git' show main:.settings | grep -o 'ada@t.co' | wc -l)\" -eq 1 ]"
assert "join refuses when the tickets directory is already there, touching nothing" \
  bash -c "cd '${people}/ada/work' && before=\"\$(git -C acme-tickets rev-parse HEAD)\" \
    && ! env ${ada} '${AMD}' join acme >/dev/null 2>&1 \
    && env ${ada} '${AMD}' join acme 2>&1 | grep -q 'acme-tickets already exists' \
    && git -C acme-tickets diff --quiet && [ \"\$(git -C acme-tickets rev-parse HEAD)\" = \"\${before}\" ]"
assert "join refuses when only the documents directory is there" \
  bash -c "mkdir -p '${people}/ada/docs-only/acme-documents' && cd '${people}/ada/docs-only' \
    && env ${ada} '${AMD}' join acme 2>&1 | grep -q 'acme-documents already exists' \
    && test ! -e acme-tickets"
assert "join refuses inside a git repository" \
  bash -c "cd '${people}/ada/work/acme-tickets' && env ${ada} '${AMD}' join acme 2>&1 | grep -q 'must not be run inside a git repository'"
assert "join refuses a project with no board, leaving no partial clones" \
  bash -c "mkdir -p '${people}/ada/blank' && cd '${people}/ada/blank' \
    && env ${ada} '${AMD}' join blank 2>&1 | grep -q \"isn't an amd project\" \
    && test -z \"\$(ls -A)\""
assert "join refuses a repository that doesn't exist, leaving nothing behind" \
  bash -c "mkdir -p '${people}/ada/missing' && cd '${people}/ada/missing' \
    && ! env ${ada} '${AMD}' join nosuch >/dev/null 2>&1 && test -z \"\$(ls -A)\""
# Four people joining at once: most pushes are rejected the first time and
# must be re-applied on top of whoever won, not dropped with a warning.
for who in bo cy di ed; do
  env_for_who="$(person "${who}" "${who}@t.co")"
  ( cd "${people}/${who}/work" && env ${env_for_who} "${AMD}" join acme >/dev/null 2>&1 ) &
done
wait
assert "four simultaneous joins all end up in the remote's users" \
  bash -c "settings=\"\$(git --git-dir='${origin}/acme/acme-tickets.git' show main:.settings)\" \
    && for who in ada bo cy di ed; do grep -q \"\${who}@t.co\" <<<\"\${settings}\" || exit 1; done"
rm -rf "${origin}" "${people}"

echo "discovery from a subdirectory:"
mkdir -p deep/nested
assert "runs from a subdir (finds repo-root board)" \
  bash -c "cd deep/nested && '${AMD}' board | grep -q TODO"

echo "discovery from a project's parent directory (not itself a git repo):"
# The amd init <project> layout: a plain folder (not a git repo itself)
# holding {project}-tickets as a child. amd board should find it without
# needing to cd one level deeper.
plain_parent="$(mktemp -d)"
mkdir -p "${plain_parent}/solo-tickets/tasks"
( cd "${plain_parent}/solo-tickets" && git init -q && git config user.email t@t.co && git config user.name t \
  && git add -A && git commit -q -m seed --allow-empty )
assert "amd board finds the one project sibling from its parent directory" \
  bash -c "cd '${plain_parent}' && '${AMD}' board --plain | grep -q TODO"
mkdir -p "${plain_parent}/second-tickets/tasks"
( cd "${plain_parent}/second-tickets" && git init -q && git config user.email t@t.co && git config user.name t )
assert "two project siblings is an ambiguity error, not a guess" \
  bash -c "cd '${plain_parent}' && '${AMD}' board 2>&1 | grep -q 'more than one project board'"
assert "an unrelated empty directory still errors as before" \
  bash -c "cd '${nongit}' && '${AMD}' board 2>&1 | grep -q 'not inside a git repository'"
# Unlike `amd init`, plain board discovery no longer registers anything on
# its own — nothing to clean up here any more.

echo "discovery from inside a sibling repo that has no board of its own (ticket 061):"
# The other real layout: standing inside the *code* repo, a git repository
# in its own right with no tasks/ of its own, sitting next to the real
# tickets repo rather than above it. locate() alone "succeeds" here
# (it never checks the board actually exists), which is exactly what used
# to stop the sibling-scan fallback from ever running.
sibling_parent="$(mktemp -d)"
( cd "${sibling_parent}" && git init -q code && cd code \
  && git config user.email t@t.co && git config user.name t && git commit -q -m seed --allow-empty )
mkdir -p "${sibling_parent}/proj-tickets/tasks"
( cd "${sibling_parent}/proj-tickets" && git init -q && git config user.email t@t.co && git config user.name t \
  && git add -A && git commit -q -m seed --allow-empty )
assert "amd board finds the sibling tickets repo from inside the code repo" \
  bash -c "cd '${sibling_parent}/code' && '${AMD}' board --plain | grep -q TODO"
mkdir -p "${sibling_parent}/code/deep/nested"
assert "...and from a subdirectory of the code repo, not just its top level" \
  bash -c "cd '${sibling_parent}/code/deep/nested' && '${AMD}' board --plain | grep -q TODO"
mkdir -p "${sibling_parent}/second-tickets/tasks"
( cd "${sibling_parent}/second-tickets" && git init -q && git config user.email t@t.co && git config user.name t )
# Two *-tickets* siblings is the `amd init`/`amd join` layout — a folder of
# projects — so from a code repository it's an error naming the fix (ticket
# 093), no longer the silent "no board" that used to hide it. Unrelated
# boards that merely share a parent (this suite's own scratch repos in
# /tmp) still fall back quietly; the 093 section below checks that.
assert "two tickets siblings from inside the code repo is an error naming amd project use" \
  bash -c "cd '${sibling_parent}/code' && '${AMD}' board --no-input 2>&1 | grep -q 'amd project use'"
rm -rf "${sibling_parent}"

lonely_parent="$(mktemp -d)"
( cd "${lonely_parent}" && git init -q lonely-code && cd lonely-code \
  && git config user.email t@t.co && git config user.name t )
assert "a code repo with no tickets sibling at all still falls back to today's behaviour" \
  bash -c "cd '${lonely_parent}/lonely-code' && '${AMD}' board --no-input 2>&1 | grep -q 'no board at'"

echo "several projects in one folder (ticket 093):"
# acme and beta side by side, each a tickets + documents pair, plus a code
# repository with no board of its own — the layout two `amd init`s make.
projects="$(mktemp -d)"
(
  set -e
  cd "${projects}"
  for name in acme beta; do
    git init -q "${name}-tickets" && git init -q "${name}-documents"
    ( cd "${name}-tickets" && git config user.email t@t.co && git config user.name t \
      && "${AMD}" init "${name}" --local-only >/dev/null \
      && AMD_COMMIT=0 "${AMD}" new "${name} work" >/dev/null )
  done
  git init -q app
)
assert "from the folder, two projects and no choice is an error naming amd project use" \
  bash -c "cd '${projects}' && ! '${AMD}' board --plain 2>/dev/null && '${AMD}' board 2>&1 | grep -q 'amd project use'"
assert "from a code repository beside them, the same error — not a silent empty board" \
  bash -c "cd '${projects}/app' && '${AMD}' board --no-input 2>&1 | grep -q 'amd project use'"
assert "amd project lists both, none current yet" \
  bash -c "cd '${projects}' && out=\"\$('${AMD}' project)\" && grep -q '^  acme ' <<<\"\${out}\" \
    && grep -q '^  beta ' <<<\"\${out}\" && grep -q 'no current project' <<<\"\${out}\""
assert "amd project use beta makes it current" \
  bash -c "cd '${projects}' && '${AMD}' project use beta | grep -q 'current project: beta' \
    && grep -qx beta .current-project && '${AMD}' project | grep -q '^\* beta '"
assert "the folder then shows beta's board" \
  bash -c "cd '${projects}' && '${AMD}' board --plain | grep -q 'beta work'"
assert "so does the code repository beside it" \
  bash -c "cd '${projects}/app' && '${AMD}' board --plain | grep -q 'beta work'"
assert "inside acme-tickets, acme's own board still wins" \
  bash -c "cd '${projects}/acme-tickets' && '${AMD}' board --plain | grep -q 'acme work'"
assert "amd project use works from inside a tickets repository too" \
  bash -c "cd '${projects}/acme-tickets' && '${AMD}' project use acme >/dev/null && grep -qx acme '${projects}/.current-project'"
assert "amd project use refuses a project that isn't here, changing nothing" \
  bash -c "cd '${projects}' && '${AMD}' project use nosuch 2>&1 | grep -q \"no project 'nosuch'\" && grep -qx acme .current-project"
assert "amd project use with no name and no terminal is a usage error" \
  bash -c "cd '${projects}' && '${AMD}' project use 2>&1 | grep -q 'usage: amd project use'"
assert ".current-project naming a project that's gone is a clear error" \
  bash -c "cd '${projects}' && echo gone >.current-project && '${AMD}' board 2>&1 | grep -q \"current project 'gone'\""
rm -f "${projects}/.current-project"
# A second `amd init` into a folder that already holds one project keeps the
# first one current, so adding a project never changes what amd shows.
second="$(mktemp -d)"
(
  set -e
  cd "${second}"
  for repo in first-tickets first-documents later-tickets later-documents; do
    git init -q "${repo}"
    git -C "${repo}" config user.email t@t.co && git -C "${repo}" config user.name t
  done
  "${AMD}" init first >/dev/null 2>&1
)
assert "init of a second project keeps the first current, and says how to switch" \
  bash -c "cd '${second}' && '${AMD}' init later 2>/dev/null | grep -q 'current is still first; .amd project use later. to switch' \
    && grep -qx first .current-project"
assert "init with a choice already made leaves it alone" \
  bash -c "cd '${second}' && echo later >.current-project && '${AMD}' init first >/dev/null 2>&1 && grep -qx later .current-project"
"${AMD}" deregister tickets >/dev/null 2>&1; "${AMD}" deregister documents >/dev/null 2>&1
for repo in first-tickets first-documents later-tickets later-documents; do
  "${AMD}" deregister "${repo}" >/dev/null 2>&1
done
rm -rf "${projects}" "${second}"

echo "create:"
"${AMD}" new "First task" >/dev/null
"${AMD}" new "Second task" -t x -t y >/dev/null
git add -A; git commit -qm seed
assert "creates tasks/backlog/001-first-task.md (new work starts in the backlog)" test -f tasks/backlog/001-first-task.md
assert "title in frontmatter" grep -q '^title: "First task"$' tasks/backlog/001-first-task.md
assert "tags in frontmatter" grep -q '^tags: \[x,y\]$' tasks/backlog/002-second-task.md
assert "a new ticket has priority low" grep -q '^priority: "low"$' tasks/backlog/002-second-task.md
assert "amd set priority changes it" \
  bash -c "'${AMD}' set 2 priority high | grep -q 'priority high' && grep -q '^priority: \"high\"\$' tasks/backlog/002-second-task.md"
assert "an invalid priority is refused" \
  bash -c "'${AMD}' set 2 priority urgent 2>&1 | grep -q 'low, medium, high' && grep -q '^priority: \"high\"\$' tasks/backlog/002-second-task.md"
assert "a new ticket has no branch type by default" \
  grep -q '^branch-type: ""$' tasks/backlog/001-first-task.md
assert "and so has no branch name" grep -q '^branch-name: ""$' tasks/backlog/001-first-task.md
assert "a new ticket is unassigned" grep -q '^assignee: ""$' tasks/backlog/001-first-task.md
assert "created date is filled in" grep -qE '^created: "[0-9]{4}-[0-9]{2}-[0-9]{2}"$' tasks/backlog/001-first-task.md

echo "parents and the tree:"
"${AMD}" new "Guest checkout" --branch-type bugfix >/dev/null
"${AMD}" new "Address form" --parent 3 >/dev/null
assert "the child records its parent id" grep -q '^parent: "003"$' tasks/backlog/004-address-form.md
assert "the child links back to the parent" \
  bash -c "grep -q '^\[\[003-guest-checkout\]\]$' tasks/backlog/004-address-form.md"
assert "the parent lists the child" \
  bash -c "grep -q '^- \[\[004-address-form\]\]$' tasks/backlog/003-guest-checkout.md"
assert "a ticket with no parent has an empty parent field" \
  bash -c "grep -q '^parent: \"\"$' tasks/backlog/001-first-task.md"
assert "an unknown parent is rejected" bash -c "! '${AMD}' new 'Orphan' --parent 99"
assert "the board nests a child under its parent" \
  bash -c "'${AMD}' board --plain | grep -q '^    \[004\] Address form'"
assert "the parent stays at the outer level" \
  bash -c "'${AMD}' board --plain | grep -q '^  \[003\] Guest checkout'"
"${AMD}" new "Deep child" --parent 4 >/dev/null
assert "nesting goes deeper than two levels" \
  bash -c "'${AMD}' board --plain | grep -q '^      \[005\] Deep child'"
assert "epics and stories are gone" \
  bash -c "! '${AMD}' epics 2>/dev/null && ! '${AMD}' stories 2>/dev/null"

echo "board:"
assert "board shows all three columns" \
  bash -c "'${AMD}' board | grep -q TODO && '${AMD}' board | grep -q DOING && '${AMD}' board | grep -q DONE"
assert "board lists the task by id and title" \
  bash -c "'${AMD}' board | grep -q '\[001\] First task'"
assert "empty columns say so" bash -c "'${AMD}' ls doing | grep -q '(empty)'"

echo "branch types:"
"${AMD}" new "Crash on save" --branch-type bugfix >/dev/null
assert "the branch type is recorded" grep -q '^branch-type: "bugfix"$' tasks/backlog/006-crash-on-save.md
assert "the branch name comes from the type and the title" \
  grep -q '^branch-name: "bugfix/crash-on-save"$' tasks/backlog/006-crash-on-save.md
assert "an unknown branch type is rejected, listing the valid ones" \
  bash -c "'${AMD}' new 'Nope' --branch-type feat 2>&1 | grep -q \"unknown branch type 'feat'\""
assert "AMD_BRANCH_TYPES overrides the list" \
  bash -c "AMD_BRANCH_TYPES='spike,chore' '${AMD}' new 'Try it' --branch-type spike >/dev/null && grep -q '^branch-name: \"spike/try-it\"\$' tasks/backlog/007-try-it.md"
assert "there is one ticket template now" \
  bash -c "'${AMD}' templates | grep -q '^ticket' && ! '${AMD}' templates | grep -q '^admin'"
assert "the board shows the branch type" \
  bash -c "'${AMD}' board --plain | grep -q 'Crash on save  (bugfix)'"

echo "moves (git mv) + branches — entering todo/ is what creates one now:"
git add -A; git commit -qm labels
"${AMD}" todo 1 >/dev/null
assert "a ticket with no branch type creates no branch" \
  bash -c "! git branch --list 'feature/first-task' | grep -q ."
assert "it still moves to todo" test -f tasks/todo/001-first-task.md
assert "todo says why it left the branch alone" \
  bash -c "'${AMD}' backlog 1 >/dev/null && '${AMD}' todo 1 2>&1 | grep -q 'no branch type on this ticket'"
git checkout -q main 2>/dev/null || git checkout -q master
"${AMD}" todo 6 >/dev/null
assert "a ticket with a branch type gets its branch" \
  bash -c "git branch --list 'bugfix/crash-on-save' | grep -q ."
"${AMD}" --no-input todo 3 --no-branch >/dev/null
assert "--no-branch leaves the branch alone" \
  bash -c "! git branch --list 'bugfix/guest-checkout' | grep -q ."
assert "todo: no-branch still moved the task" test -f tasks/todo/003-guest-checkout.md
"${AMD}" backlog 3 >/dev/null
"${AMD}" todo 3 --branch spike/try-something >/dev/null
assert "--branch overrides the ticket" \
  bash -c "git branch --show-current | grep -q '^spike/try-something$'"
assert "an invalid --branch is rejected" \
  bash -c "'${AMD}' todo 5 --branch 'bad branch' 2>&1 | grep -q 'cannot contain whitespace'"
git checkout -q main 2>/dev/null || git checkout -q master
assert "doing is a plain move now: no branch effect at all" \
  bash -c "'${AMD}' doing 1 >/dev/null && test -f tasks/doing/001-first-task.md"
AMD_COMMIT=0 "${AMD}" "done" 1 >/dev/null
assert "done: doing -> done" test -f tasks/done/001-first-task.md
assert "moved via git (rename tracked)" bash -c 'git status --porcelain | grep -q "^R"'
# The move message comes from the CLI now, not from the library (ADR-0004).
assert "a move reports where the ticket went" \
  bash -c "'${AMD}' todo 2 --no-branch | grep -q 'moved 002-second-task.md -> todo/'"
assert "a ticket can't skip a column" \
  bash -c "'${AMD}' done 2 2>&1 | grep -q 'one column at a time' && test -f tasks/todo/002-second-task.md"
assert "moving into the same column fails" bash -c "! '${AMD}' done 1"
assert "amd start no longer exists" bash -c "! '${AMD}' start 1 2>/dev/null"

echo "destination verbs (move + list, ticket 002):"
assert "amd todo <ref> moves it to todo/" \
  bash -c "'${AMD}' todo 4 >/dev/null && test -f tasks/todo/004-address-form.md"
assert "amd doing <ref> moves it to doing/" \
  bash -c "'${AMD}' doing 4 >/dev/null && test -f tasks/doing/004-address-form.md"
assert "amd backlog <ref> moves it back to backlog/, a column at a time" \
  bash -c "'${AMD}' todo 4 --no-branch >/dev/null && '${AMD}' backlog 4 >/dev/null && test -f tasks/backlog/004-address-form.md"
assert "amd todo with no ref lists todo/, same as amd ls todo" \
  bash -c "diff <('${AMD}' todo) <('${AMD}' ls todo) >/dev/null"
assert "amd done with no ref lists done/, same as amd ls done" \
  bash -c "diff <('${AMD}' done) <('${AMD}' ls done) >/dev/null"
assert "amd board and amd ls are unaffected by the no-ref list form" \
  bash -c "diff <('${AMD}' board) <('${AMD}' ls) >/dev/null"
assert "an unknown destination names what is valid" \
  bash -c "'${AMD}' retest-tui 4 2>&1 | grep -q \"'retest-tui' is not a column, 'abandon', or an epic on this board\""
assert "the error lists the columns and says there are no epics" \
  bash -c "'${AMD}' retest-tui 4 2>&1 | grep -q 'columns: backlog, todo, doing, done; epics: none'"
assert "amd back no longer exists" bash -c "! '${AMD}' back 4 2>/dev/null"

echo "refs + ids:"
"${AMD}" todo second >/dev/null
assert "find by slug substring" test -f tasks/todo/002-second-task.md
git checkout -q main 2>/dev/null || git checkout -q master
"${AMD}" new "Third" >/dev/null
assert "ids continue across columns (008)" test -f tasks/backlog/008-third.md
assert "unknown ref fails" bash -c "! '${AMD}' show 99"
assert "ambiguous ref fails" bash -c "! '${AMD}' show task"
assert "show prints the file" bash -c "'${AMD}' show 008 | grep -q '^title: \"Third\"$'"

echo "templates:"
assert "templates says where each ticket type comes from" \
  bash -c "'${AMD}' templates | grep -q '^ticket  *built-in$'"
assert "unknown template fails with a hint" \
  bash -c "'${AMD}' new 'Nope' --template nope 2>&1 | grep -q 'amd templates'"
assert "eject writes an editable copy into the board" \
  bash -c "'${AMD}' templates eject ticket >/dev/null && test -f tasks/templates/ticket.md.jinja"
assert "eject refuses to clobber without --force" \
  bash -c "! '${AMD}' templates eject ticket"
printf -- '---\nid: {{ id | yaml }}\ntitle: {{ title | yaml }}\ntype: {{ branch_type | yaml }}\nassignee: {{ extra.owner | yaml }}\n---\n\n## Custom\n' \
  > tasks/templates/ticket.md.jinja
assert "board template overrides the built-in" \
  bash -c "'${AMD}' new 'Overridden' -s owner=tim >/dev/null && grep -q '^## Custom$' tasks/backlog/*-overridden.md"
assert "--set values reach the template" \
  bash -c "grep -q '^assignee: \"tim\"$' tasks/backlog/*-overridden.md"
assert "labels reach a custom template" \
  bash -c "grep -q '^type: \"\"$' tasks/backlog/*-overridden.md"
assert "templates list shows the board override" \
  bash -c "'${AMD}' templates | grep -q 'templates/ticket.md.jinja'"
printf -- '---\nid: {{ id | yaml }}\nassignee: {{ extra.owner | yaml }}\ndue: {{ extra["due date"] | yaml }}\n---\n' \
  > tasks/templates/ticket.md.jinja
assert "a template field with no value is an error, naming the flag" \
  bash -c "'${AMD}' new 'Needs fields' 2>&1 | grep -q -- '--set owner='"
assert "every missing field is listed" \
  bash -c "'${AMD}' new 'Needs fields' 2>&1 | grep -q -- '--set due date='"
printf 'title: {{ titel }}\n' > tasks/templates/ticket.md.jinja
assert "a typo'd variable is an error, not a blank line" \
  bash -c "! '${AMD}' new 'Broken' 2>&1 | grep -q '^title: $'"
assert "the template error names the template and line" \
  bash -c "'${AMD}' new 'Broken' 2>&1 | grep -q \"template 'ticket'\""
rm -rf tasks/templates

echo "the body editor:"
assert "non-interactive create never opens an editor" \
  bash -c "EDITOR=false '${AMD}' new 'No editor here' >/dev/null && test -f tasks/backlog/*-no-editor-here.md"
assert "--no-edit is accepted alongside a title" \
  bash -c "'${AMD}' new 'Also no editor' --no-edit >/dev/null"
assert "--edit and --no-edit conflict" \
  bash -c "! '${AMD}' new 'Both' --edit --no-edit"

echo "quoting:"
"${AMD}" new 'Fix the "quoted" thing' >/dev/null
assert "a quote in the title is escaped, not left to break the frontmatter" \
  bash -c "grep -qF 'title: \"Fix the \\\"quoted\\\" thing\"' tasks/backlog/*-fix-the-quoted-thing.md"

echo "the archive:"
assert "init creates an archive that keeps itself out of git" \
  bash -c "test -f tasks/archive/.gitignore && grep -q '^\*$' tasks/archive/.gitignore"
"${AMD}" new "Bin this one" >/dev/null
git add -A; git commit -qm junkable
assert "abandon takes a ticket off the board" \
  bash -c "AMD_COMMIT=0 '${AMD}' abandon bin-this-one | grep -q 'archived'"
assert "the ticket is in the archive" test -f tasks/archive/*-bin-this-one.md
assert "amd abandon with no ref lists the archive" \
  bash -c "'${AMD}' abandon | grep -q 'Bin this one'"
assert "the board no longer shows it" \
  bash -c "! '${AMD}' board --plain | grep -q 'Bin this one'"
assert "git records the ticket leaving the board" \
  bash -c "git status --porcelain | grep -q '^D  tasks/backlog/.*bin-this-one'"
assert "the archive stays out of git" bash -c "! git status --porcelain | grep -q 'tasks/archive/0'"
assert "an archived ticket is no longer findable" bash -c "! '${AMD}' show bin-this-one"
assert "ids are not reused after archiving" \
  bash -c "before=\$(ls tasks/archive | head -1 | cut -d- -f1); '${AMD}' new 'After the bin' >/dev/null; ! test -f tasks/backlog/\${before}-after-the-bin.md"
assert "the counter records the next id, in .settings, not tasks/" \
  bash -c "test ! -f tasks/.next-id && test \"\$(grep '^next-id' .settings | sed 's/.*= //')\" -gt \"\$(ls tasks/backlog | tail -1 | cut -d- -f1 | sed 's/^0*//')\""
assert "a deleted ticket does not hand its id back" \
  bash -c "'${AMD}' new 'Doomed' >/dev/null; id=\$(ls tasks/backlog | grep doomed | cut -d- -f1); rm tasks/backlog/\${id}-doomed.md; '${AMD}' new 'After the delete' >/dev/null; ! test -f tasks/backlog/\${id}-after-the-delete.md"

echo "branch cleanup on abandon:"
"${AMD}" new "Cleanup candidate" --branch-type feature >/dev/null
cid=$(basename tasks/backlog/*-cleanup-candidate.md | cut -d- -f1)
"${AMD}" todo "${cid}" >/dev/null
assert "the branch exists once the ticket enters todo" \
  bash -c "git branch --list 'feature/cleanup-candidate' | grep -q ."
git checkout -q main 2>/dev/null || git checkout -q master
assert "abandoning an untouched branch deletes it too" \
  bash -c "'${AMD}' abandon ${cid} | grep -q 'deleted feature/cleanup-candidate'"
assert "the branch is actually gone" \
  bash -c "! git branch --list 'feature/cleanup-candidate' | grep -q ."

"${AMD}" new "Cleanup keeps real work" --branch-type feature >/dev/null
kid=$(basename tasks/backlog/*-cleanup-keeps-real-work.md | cut -d- -f1)
# Commit the ticket to the base branch first: otherwise it only exists on the
# feature branch's own history, and checking back out to main makes it vanish
# from the working tree (same trap the board.rs unit test hit).
git add -A; git commit -qm "add cleanup-keeps-real-work ticket"
"${AMD}" todo "${kid}" >/dev/null
echo "real work" > realwork.txt
git add -A; git commit -qm "real work on the branch"
git checkout -q main 2>/dev/null || git checkout -q master
assert "abandoning a branch with real commits leaves it alone" \
  bash -c "out=\$('${AMD}' abandon ${kid}); echo \"\$out\" | grep -q archived && ! echo \"\$out\" | grep -q deleted"
assert "the branch with real work survives" \
  bash -c "git branch --list 'feature/cleanup-keeps-real-work' | grep -q ."

echo "assignees:"
assert "a new ticket is unassigned" \
  bash -c "grep -q '^assignee: \"\"$' tasks/*/001-first-task.md"
"${AMD}" new "Assigned at creation" -a alex >/dev/null
assert "--assignee sets it at creation" \
  bash -c "grep -q '^assignee: \"alex\"$' tasks/backlog/*-assigned-at-creation.md"
assert "amd assign sets it afterwards" \
  bash -c "'${AMD}' assign 1 sam >/dev/null && grep -q '^assignee: \"sam\"$' tasks/*/001-first-task.md"
assert "amd assign with no name clears it" \
  bash -c "'${AMD}' assign 1 >/dev/null && grep -q '^assignee: \"\"$' tasks/*/001-first-task.md"
assert "@me resolves to the git user" \
  bash -c "'${AMD}' assign 1 @me >/dev/null && grep -q '^assignee: \"t\"$' tasks/*/001-first-task.md"
assert "the board shows the assignee" bash -c "'${AMD}' board --plain | grep -q '@t'"

echo "registration:"
tmp_name="$(basename "${tmp}")"
# periods_and_underscores was deregistered earlier (right after its own
# init --local-only) — reused here specifically because it's a real board
# that's confirmed *not* currently registered, to check that merely running
# a board-resolving command doesn't silently re-add it.
assert "amd board no longer registers anything on its own" \
  bash -c "cd '${periods_and_underscores}' && '${AMD}' board >/dev/null; \
    ! grep -q \"$(basename "${periods_and_underscores}")\$\" \"\${XDG_CONFIG_HOME}/agile-md/repos\" 2>/dev/null"
assert "amd register declares the repo for the project and registers it locally" \
  bash -c "'${AMD}' register | grep -q 'registered' \
    && grep -qE \"^repos = .*${tmp_name}\" .settings \
    && grep -q \"${tmp_name}\$\" \"\${XDG_CONFIG_HOME}/agile-md/repos\""
assert "registering the same path again is a local no-op, not an error" \
  bash -c "'${AMD}' register | grep -q 'already registered on this machine'"
assert "a non-repository is refused" \
  bash -c "'${AMD}' register '${nongit}' 2>&1 | grep -q 'not a git repository'"
assert "amd deregister removes only the local registration" \
  bash -c "'${AMD}' deregister \"${tmp_name}\" | grep -q '^deregistered ' \
    && ! grep -q \"${tmp_name}\$\" \"\${XDG_CONFIG_HOME}/agile-md/repos\""
assert "the project-wide declaration survives deregistering locally" \
  bash -c "grep -qE \"^repos = .*${tmp_name}\" .settings"
assert "deregistering something not registered locally fails" \
  bash -c "! '${AMD}' deregister nope"

echo "aliases:"
pu_name="$(basename "${periods_and_underscores}")"
alias_other="$(mktemp -d)"
(cd "${alias_other}" && git init -q)
other_name="$(basename "${alias_other}")"
"${AMD}" register "${periods_and_underscores}" >/dev/null
"${AMD}" register "${alias_other}" >/dev/null
assert "amd alias assigns a short name to a registered repository" \
  bash -c "'${AMD}' alias \"${pu_name}\" pnick | grep -q '^pnick -> '"
assert "aliasing an unregistered repository is refused" \
  bash -c "! '${AMD}' alias not-a-real-repo somealias"
assert "an alias cannot collide with a different repo's real registered name" \
  bash -c "! '${AMD}' alias \"${pu_name}\" \"${other_name}\""
assert "amd deregister resolves an alias to its real name" \
  bash -c "'${AMD}' deregister pnick | grep -q \"deregistered ${pu_name}\""
rm -rf "${alias_other}"

echo "related tickets:"
assert "related is empty by default" \
  bash -c "grep -q '^related: \[\]$' tasks/*/001-first-task.md"
"${AMD}" new "Depends on the first" --related 1 >/dev/null
assert "--related records the id" \
  bash -c "grep -q '^related: \[001\]$' tasks/backlog/*-depends-on-the-first.md"
assert "the other end is linked back" \
  bash -c "grep -qE '^related: \[0[0-9]+\]$' tasks/*/001-first-task.md"
assert "an unknown related ref is rejected at creation" \
  bash -c "! '${AMD}' new 'Dangling' --related 99"
assert "amd link relates two tickets both ways" \
  bash -c "'${AMD}' link 2 depends-on-the-first >/dev/null \
    && grep -q 'related: \[0' tasks/*/002-second-task.md \
    && grep -q '002' tasks/backlog/*-depends-on-the-first.md"
assert "amd link is idempotent" \
  bash -c "'${AMD}' link 2 depends-on-the-first | grep -q 'already related'"
assert "a task cannot be related to itself" \
  bash -c "'${AMD}' link 2 2 2>&1 | grep -q \"can't be related to itself\""
"${AMD}" new "One way only" >/dev/null
assert "amd link --one-way leaves the other end alone" \
  bash -c "'${AMD}' link 2 one-way --one-way >/dev/null && ! grep -q '002' tasks/backlog/*-one-way-only.md"
assert "a ticket with no branch carries the list too" \
  bash -c "grep -q '^related: \[\]$' tasks/*/*-assigned-at-creation.md"

# `amd set` and `amd group` shipped with no coverage here, which is how the
# suite came to disagree with the tool. They get their own repository so the
# epic and sprint directories cannot disturb the id sequencing above.
echo "set:"
work="$(mktemp -d)"
( cd "${work}" && git init -q && git config user.email t@t.co && git config user.name t \
  && AMD_YES=1 "${AMD}" init gadgets --local-only >/dev/null \
  && "${AMD}" new "First" >/dev/null && "${AMD}" new "Second" >/dev/null )
assert "set points sizes a ticket" \
  bash -c "cd '${work}' && '${AMD}' set 001 points 5 | grep -q 'sized 5' && grep -q '^points: \"5\"\$' tasks/backlog/001-first.md"
assert "set points rejects a non-Fibonacci value" \
  bash -c "cd '${work}' && '${AMD}' set 001 points 4 2>&1 | grep -q 'invalid points'"
assert "set points clears with an empty value, unaffected by the Fibonacci check" \
  bash -c "cd '${work}' && '${AMD}' set 001 points '' | grep -q 'sized' && grep -q '^points: \"\"\$' tasks/backlog/001-first.md"
( cd "${work}" && "${AMD}" set 001 points 5 >/dev/null )
assert "set title rewrites the title, not the filename" \
  bash -c "cd '${work}' && '${AMD}' set 001 title 'Renamed thing' | grep -q 'retitled' && grep -q '^title: \"Renamed thing\"\$' tasks/backlog/001-first.md"
assert "set order ranks a ticket fractionally" \
  bash -c "cd '${work}' && '${AMD}' set 002 order 1.5 | grep -q 'ranked 1.5' && grep -q '^order: \"1.5\"\$' tasks/backlog/002-second.md"
assert "an unknown field is rejected" \
  bash -c "cd '${work}' && ! '${AMD}' set 001 nonsense 1"

echo "epics and sprints:"
assert "an empty backlog says so" \
  bash -c "cd '${work}' && '${AMD}' group list | grep -q 'no epics or sprints'"
assert "group epic creates one" \
  bash -c "cd '${work}' && '${AMD}' group epic checkout --description 'the checkout flow' | grep -q 'created epic checkout' && test -f tasks/backlog/checkout/_group.md"
assert "group sprint records its length" \
  bash -c "cd '${work}' && '${AMD}' group sprint sprint-1 --days 10 | grep -q 'created sprint sprint-1 (10 days)'"
assert "a duplicate group is refused" \
  bash -c "cd '${work}' && ! '${AMD}' group epic checkout"
assert "set epic files the ticket into the epic directory" \
  bash -c "cd '${work}' && '${AMD}' set 002 epic checkout | grep -q 'filed under checkout' && test -f tasks/backlog/checkout/002-second.md && grep -q '^epic: \"checkout\"\$' tasks/backlog/checkout/002-second.md"
# Sizing keys are written on demand, not stubbed at creation — an unsized
# ticket has no `points:` line at all, which is what an epic accepts and a
# sprint refuses.
assert "an epic takes an unsized ticket" \
  bash -c "cd '${work}' && ! grep -q '^points:' tasks/backlog/checkout/002-second.md"
assert "a sprint refuses an unsized ticket" \
  bash -c "cd '${work}' && '${AMD}' set 002 epic sprint-1 2>&1 | grep -q 'needs points before it can go in sprint-1'"
assert "a sprint takes a sized one" \
  bash -c "cd '${work}' && '${AMD}' set 001 epic sprint-1 | grep -q 'filed under sprint-1' && test -f tasks/backlog/sprint-1/001-first.md"
assert "group list totals tickets and points" \
  bash -c "cd '${work}' && '${AMD}' group list | grep -qE 'sprint-1 +sprint +10d +pending +1 ticket\(s\), 5 point\(s\)'"
assert "starting a sprint says so" \
  bash -c "cd '${work}' && '${AMD}' group start sprint-1 | grep -q 'started'"
# A started sprint is a commitment (ticket 083): it takes nothing more and
# gives nothing back — its tickets only move on, to todo.
assert "a started sprint gives nothing back" \
  bash -c "cd '${work}' && '${AMD}' set 001 epic checkout 2>&1 | grep -q 'only move on to todo'"
assert "a started sprint takes nothing more, even sized" \
  bash -c "cd '${work}' && '${AMD}' set 002 points 3 >/dev/null && '${AMD}' set 002 epic sprint-1 2>&1 | grep -q 'no more tickets'"
assert "a started sprint refuses an unsized ticket too" \
  bash -c "cd '${work}' && '${AMD}' new 'Unsized' >/dev/null && ! '${AMD}' set 003 epic sprint-1 >/dev/null 2>&1"
assert "archiving straight out of a started sprint is refused" \
  bash -c "cd '${work}' && '${AMD}' abandon 001 2>&1 | grep -q 'move it on to todo before archiving'"
assert "a ticket outside the sprint is archived" \
  bash -c "cd '${work}' && '${AMD}' abandon 002 | grep -q 'archived'"
assert "amd abandon with no ref lists the archive" \
  bash -c "cd '${work}' && '${AMD}' abandon | grep -q Second"
# The bug this guards: a sprint's points fell as work progressed, because the
# count scanned backlog/ only and a ticket in doing/ has no epic directory.
assert "a sprint keeps its points when work starts" \
  bash -c "cd '${work}' && before=\$('${AMD}' group list | grep sprint-1) && '${AMD}' todo 001 --no-branch >/dev/null && '${AMD}' doing 001 >/dev/null && after=\$('${AMD}' group list | grep sprint-1) && [ \"\${before}\" = \"\${after}\" ]"
assert "amd group rename renames a group and relabels its tickets" \
  bash -c "cd '${work}' && '${AMD}' group rename checkout payments | grep -q 'renamed checkout -> payments' \
    && test -d tasks/backlog/payments && ! test -d tasks/backlog/checkout \
    && ! grep -rq --exclude-dir=archive '^epic: \"checkout\"' tasks && '${AMD}' group list | grep -q payments \
    && '${AMD}' group rename payments checkout >/dev/null"
assert "starting an unknown group fails" \
  bash -c "cd '${work}' && ! '${AMD}' group start nope"
assert "amd <epic> <ref> files a ticket, same as set epic" \
  bash -c "cd '${work}' && '${AMD}' new 'Verb filed' >/dev/null \
    && id=\$(ls tasks/backlog | grep verb-filed | cut -d- -f1) \
    && '${AMD}' checkout \${id} | grep -q 'filed under checkout' \
    && test -f tasks/backlog/checkout/\${id}-verb-filed.md"
assert "amd <epic> with no ref lists that epic's tickets" \
  bash -c "cd '${work}' && '${AMD}' checkout | grep -q 'Verb filed'"
assert "amd <epic> --tag narrows an epic listing to tasks carrying that tag" \
  bash -c "cd '${work}' && '${AMD}' new 'Verb filed and urgent' -t urgent --no-edit >/dev/null \
    && id=\$(ls tasks/backlog | grep verb-filed-and-urgent | cut -d- -f1) \
    && '${AMD}' checkout \${id} >/dev/null \
    && '${AMD}' checkout --tag urgent | grep -q 'Verb filed and urgent' \
    && ! '${AMD}' checkout --tag urgent | grep -q '\[.*\] Verb filed\$'"
assert "an epic cannot be created with a column's name" \
  bash -c "cd '${work}' && '${AMD}' group epic todo 2>&1 | grep -q collides"
assert "an epic cannot be created named abandon" \
  bash -c "cd '${work}' && '${AMD}' group epic abandon 2>&1 | grep -q collides"
assert "amd group empty moves every ticket in a group back to loose backlog, no precondition" \
  bash -c "cd '${work}' && '${AMD}' --plain checkout | grep -q '\[' \
    && '${AMD}' group empty checkout | grep -q 'emptied into backlog' \
    && '${AMD}' --plain checkout | grep -q '(empty)'"
assert "a nonempty group cannot be deleted" \
  bash -c "cd '${work}' && '${AMD}' new 'Back in checkout' --no-edit >/dev/null \
    && id=\$(ls tasks/backlog | grep back-in-checkout | cut -d- -f1) \
    && '${AMD}' checkout \${id} >/dev/null \
    && '${AMD}' group delete checkout 2>&1 | grep -q '1 ticket' \
    && test -d tasks/backlog/checkout"
assert "an emptied group can be deleted" \
  bash -c "cd '${work}' && '${AMD}' group empty checkout >/dev/null \
    && '${AMD}' group delete checkout | grep -q 'deleted checkout' \
    && ! test -d tasks/backlog/checkout"
assert "deleting an unknown group is an error" \
  bash -c "cd '${work}' && ! '${AMD}' group delete nope"
rm -rf "${work}"

echo "commit-and-push:"
commit_repo="$(mktemp -d)"
commit_origin="$(mktemp -d)/origin.git"
mkdir -p "${commit_origin}" && git init -q --bare "${commit_origin}"
( cd "${commit_repo}" && git init -q -b main && git config user.email t@t.co && git config user.name t \
  && git remote add origin "${commit_origin}" \
  && AMD_YES=1 "${AMD}" init committest --local-only >/dev/null )
assert "amd new commits both the new ticket and .settings' next-id bump" \
  bash -c "cd '${commit_repo}' && '${AMD}' new 'First ticket' --no-edit >/dev/null \
    && git show --stat --pretty=format: HEAD | grep -q '001-first-ticket.md' \
    && git show --stat --pretty=format: HEAD | grep -q '.settings'"
assert "that commit was pushed to origin" \
  bash -c "cd '${commit_repo}' && [ \"\$(git rev-parse HEAD)\" = \"\$(git rev-parse origin/main)\" ]"
assert "a file staged by hand beforehand is never swept into the next commit" \
  bash -c "cd '${commit_repo}' && echo dummy > dummy.md && git add dummy.md \
    && '${AMD}' todo 1 --no-branch >/dev/null \
    && ! git show --stat --pretty=format: HEAD | grep -q dummy.md \
    && git status --porcelain=v1 | grep -q '^A  dummy.md\$'"
assert "AMD_COMMIT=0 suppresses the commit entirely" \
  bash -c "cd '${commit_repo}' && AMD_COMMIT=0 '${AMD}' doing 1 >/dev/null \
    && git status --porcelain=v1 | grep -q '001-first-ticket.md'"
assert "the move itself still happened despite AMD_COMMIT=0" \
  bash -c "cd '${commit_repo}' && test -f tasks/doing/001-first-ticket.md"
assert "amd group empty commits every ticket it moves in one commit" \
  bash -c "cd '${commit_repo}' && '${AMD}' group epic squad >/dev/null \
    && '${AMD}' squad 1 >/dev/null && '${AMD}' new 'Second ticket' --no-edit >/dev/null \
    && '${AMD}' squad 2 >/dev/null \
    && before=\$(git rev-list --count HEAD) \
    && '${AMD}' group empty squad >/dev/null \
    && after=\$(git rev-list --count HEAD) \
    && [ \$((after - before)) -eq 1 ] \
    && git show --stat --pretty=format: HEAD | grep -q '001-first-ticket.md' \
    && git show --stat --pretty=format: HEAD | grep -q '002-second-ticket.md'"
assert "amd log renders a transition with a real sha" \
  bash -c "cd '${commit_repo}' && '${AMD}' log | grep -qE '^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}  add .*[0-9a-f]{7,}\$'"
rm -rf "${commit_repo}"

echo "WIP limits (doing only, ticket 083):"
wip_repo="$(mktemp -d)"
( cd "${wip_repo}" && git init -q && git config user.email t@t.co && git config user.name t \
  && AMD_YES=1 "${AMD}" init wiptest --local-only >/dev/null \
  && for t in First Second Third Fourth Fifth; do "${AMD}" new "${t}" --no-edit >/dev/null; done \
  && for n in 1 2 3; do "${AMD}" todo "${n}" --no-branch >/dev/null; done \
  && "${AMD}" doing 1 >/dev/null && "${AMD}" doing 2 >/dev/null )
assert "todo/ is never limited, whatever the settings say" \
  bash -c "cd '${wip_repo}' && AMD_WIP_TODO=1 '${AMD}' todo 4 --no-branch >/dev/null && test -f tasks/todo/004-fourth.md"
assert "doing/ defaults to a WIP limit of 2 with nothing configured at all" \
  bash -c "cd '${wip_repo}' && '${AMD}' doing 3 2>&1 | grep -q 'doing/ already has 2 of 2 allowed'"
assert "the refused move never happened" \
  bash -c "cd '${wip_repo}' && test -f tasks/todo/003-third.md"
assert "amd doing --force overrides the WIP limit" \
  bash -c "cd '${wip_repo}' && '${AMD}' doing 3 --force >/dev/null && test -f tasks/doing/003-third.md"
assert "moving something out of an over-limit column is never blocked" \
  bash -c "cd '${wip_repo}' && '${AMD}' done 3 >/dev/null && test -f tasks/done/003-third.md"
assert "amd wip-limit sets an explicit limit, persisted in .settings" \
  bash -c "cd '${wip_repo}' && '${AMD}' wip-limit 3 >/dev/null && grep -q '^wip-limit = 3\$' .settings"
assert "the new limit takes effect immediately" \
  bash -c "cd '${wip_repo}' && '${AMD}' doing 4 >/dev/null && test -f tasks/doing/004-fourth.md"
assert "amd wip-limit 0 turns doing/'s limit off" \
  bash -c "cd '${wip_repo}' && '${AMD}' wip-limit 0 >/dev/null \
    && '${AMD}' todo 5 --no-branch >/dev/null && '${AMD}' doing 5 >/dev/null \
    && test -f tasks/doing/005-fifth.md"
assert "amd wip-limit rejects a non-numeric value" \
  bash -c "cd '${wip_repo}' && ! '${AMD}' wip-limit nope >/dev/null 2>&1"
assert "a garbled wip-limit value in .settings resets to the default of 2, not unlimited" \
  bash -c "cd '${wip_repo}' && sed -i 's/^wip-limit = 0\$/wip-limit = nope/' .settings \
    && '${AMD}' new \"Sixth\" --no-edit >/dev/null && '${AMD}' todo 6 --no-branch >/dev/null \
    && '${AMD}' doing 6 2>&1 | grep -q 'doing/ already has 4 of 2 allowed'"
rm -rf "${wip_repo}"

echo "import and tag filtering:"
import_src="$(mktemp -d)"
mkdir -p "${import_src}/backlog" "${import_src}/todo" "${import_src}/done"
cat > "${import_src}/backlog/010-old-backlog-ticket.md" <<'TICKET'
---
id: "010"
title: "Old backlog ticket"
parent: ""
related: []
tags: []
---

body
TICKET
cat > "${import_src}/todo/003-in-progress-work.md" <<'TICKET'
---
id: "003"
title: "In progress work"
parent: ""
related: []
tags: []
---

body
TICKET
cat > "${import_src}/done/007-shipped-feature.md" <<'TICKET'
---
id: "007"
title: "Shipped feature"
parent: ""
related: []
tags: []
---

body
TICKET

import_dst="$(mktemp -d)"
( cd "${import_dst}" && git init -q && git config user.email t@t.co && git config user.name t \
  && AMD_YES=1 "${AMD}" init imported --local-only >/dev/null )
assert "amd import lands tickets in their original columns with fresh ids" \
  bash -c "cd '${import_dst}' && '${AMD}' import '${import_src}' >/dev/null \
    && test -f tasks/backlog/001-old-backlog-ticket.md \
    && test -f tasks/todo/002-in-progress-work.md \
    && test -f tasks/done/003-shipped-feature.md"
assert "the source tree is untouched by the import" \
  test -f "${import_src}/backlog/010-old-backlog-ticket.md"
assert "a --dry-run import writes nothing" \
  bash -c "cd '${import_dst}' && '${AMD}' import '${import_src}' --dry-run | grep -q 'would import 3 tickets' \
    && ! test -f tasks/backlog/004-old-backlog-ticket.md"
assert "importing the same source again flags every ticket DUPLICATE against the existing board" \
  bash -c "cd '${import_dst}' && '${AMD}' import '${import_src}' >/dev/null \
    && '${AMD}' tag DUPLICATE | grep -q 'Old backlog ticket' \
    && '${AMD}' tag DUPLICATE | grep -q 'In progress work' \
    && '${AMD}' tag DUPLICATE | grep -q 'Shipped feature'"
assert "amd tag finds nothing for a tag nobody has" \
  bash -c "cd '${import_dst}' && ! '${AMD}' tag NO-SUCH-TAG | grep -q 'Old backlog ticket'"
assert "amd backlog --tag narrows the listed column to tasks carrying that tag" \
  bash -c "cd '${import_dst}' && test \"\$('${AMD}' backlog --tag DUPLICATE --plain | grep -c 'Old backlog ticket')\" = 1 \
    && test \"\$('${AMD}' backlog --plain | grep -c 'Old backlog ticket')\" = 2"
assert "amd board --tag narrows every column to tasks carrying that tag" \
  bash -c "cd '${import_dst}' && out=\$('${AMD}' board --tag DUPLICATE --plain) \
    && echo \"\$out\" | grep -q 'Old backlog ticket' \
    && echo \"\$out\" | grep -q 'In progress work' \
    && echo \"\$out\" | grep -q 'Shipped feature' \
    && test \"\$(echo \"\$out\" | grep -c 'Old backlog ticket')\" = 1"
assert "amd ls all --tag behaves the same as amd board --tag" \
  bash -c "cd '${import_dst}' && test \"\$('${AMD}' ls all --tag DUPLICATE --plain | grep -c 'Shipped feature')\" = 1"
assert "amd <destination> --tag with no matches lists an empty column, not an error" \
  bash -c "cd '${import_dst}' && '${AMD}' backlog --tag NO-SUCH-TAG --plain | grep -q '(empty)'"

echo "locking:"
lock_repo="$(mktemp -d)"
( cd "${lock_repo}" && git init -q && git config user.email t@t.co && git config user.name t \
  && AMD_YES=1 "${AMD}" init locktest --local-only >/dev/null \
  && "${AMD}" new "Lockable ticket" --no-edit >/dev/null \
  && "${AMD}" lock 1 >/dev/null )
assert "amd lock records the holder in the ticket's own frontmatter" \
  bash -c "cd '${lock_repo}' && '${AMD}' show 1 | grep -q 'locked-by: \"t@t.co\"'"
assert "a different git identity is refused editing a leased ticket" \
  bash -c "cd '${lock_repo}' && git config user.email other@example.com && '${AMD}' set 1 points 3 2>&1 | grep -q 'leased by t@t.co'"
assert "a different git identity is refused moving a leased ticket" \
  bash -c "cd '${lock_repo}' && git config user.email other@example.com && '${AMD}' todo 1 --no-branch 2>&1 | grep -q 'leased by t@t.co'"
assert "the holder's own identity can still edit the leased ticket" \
  bash -c "cd '${lock_repo}' && git config user.email t@t.co && '${AMD}' set 1 points 3 | grep -q 'sized 3'"
assert "a different identity cannot plain-unlock someone else's lease" \
  bash -c "cd '${lock_repo}' && git config user.email other@example.com && '${AMD}' unlock 1 2>&1 | grep -q 'leased by t@t.co'"
assert "a different git identity is refused filing a leased ticket under an epic" \
  bash -c "cd '${lock_repo}' && git config user.email t@t.co && '${AMD}' group epic checkout >/dev/null \
    && git config user.email other@example.com && '${AMD}' checkout 1 2>&1 | grep -q 'leased by t@t.co'"
assert "a different git identity is refused abandoning a leased ticket" \
  bash -c "cd '${lock_repo}' && git config user.email other@example.com && '${AMD}' abandon 1 2>&1 | grep -q 'leased by t@t.co'"
assert "unlock --force breaks someone else's lease" \
  bash -c "cd '${lock_repo}' && git config user.email other@example.com && '${AMD}' unlock 1 --force | grep -q 'lease broken'"
assert "the lease is gone after being broken" \
  bash -c "cd '${lock_repo}' && ! '${AMD}' show 1 | grep -q 'locked-by: \"t@t.co\"'"
assert "amd todo auto-takes a now-free lease on entry" \
  bash -c "cd '${lock_repo}' && git config user.email t@t.co && '${AMD}' todo 1 --no-branch >/dev/null \
    && '${AMD}' show 1 | grep -q 'locked-by: \"t@t.co\"'"

echo "pipes:"
assert "the board survives a closed pipe" \
  bash -c "'${AMD}' board --plain | head -2 >/dev/null 2>/tmp/amd-pipe.err; ! grep -q 'panicked' /tmp/amd-pipe.err"
assert "showing a task survives a closed pipe" \
  bash -c "'${AMD}' show 1 | head -1 >/dev/null 2>/tmp/amd-pipe.err; ! grep -q 'panicked' /tmp/amd-pipe.err"

echo "completions:"
assert "bash completions are valid bash" \
  bash -c "'${AMD}' completions bash 2>/dev/null > comp.bash && bash -n comp.bash"
assert "zsh completions start with the compdef line" \
  bash -c "'${AMD}' completions zsh 2>/dev/null | head -1 | grep -q '^#compdef amd$'"
assert "fish completions are fish syntax" \
  bash -c "'${AMD}' completions fish 2>/dev/null | grep -q '^complete -c amd'"
assert "completions know the subcommands" \
  bash -c "'${AMD}' completions bash 2>/dev/null | grep -q 'link' && '${AMD}' completions fish 2>/dev/null | grep -q 'templates'"
assert "completions offer the change types" \
  bash -c "'${AMD}' completions fish 2>/dev/null | grep -q 'feat'"
assert "completions offer the branch types" \
  bash -c "'${AMD}' completions fish 2>/dev/null | grep -q 'bugfix'"
assert "completions offer the set fields" \
  bash -c "'${AMD}' completions bash 2>/dev/null | grep -q 'points epic order title'"
assert "the shell is taken from \$SHELL when not given" \
  bash -c "SHELL=/bin/zsh '${AMD}' completions 2>/dev/null | head -1 | grep -q '^#compdef amd$'"
assert "an unknown shell errors with the choices" \
  bash -c "SHELL=/bin/nope '${AMD}' completions 2>&1 | grep -q 'amd completions bash|zsh|fish'"
assert "the install hint goes to stderr, not into the script" \
  bash -c "'${AMD}' completions bash 2>/dev/null | grep -qv '^# install it with:' && '${AMD}' completions bash 2>&1 >/dev/null | grep -q 'install it with'"
assert "completions work without a board" \
  bash -c "cd '${nongit}' && '${AMD}' completions bash | grep -q '_amd'"
rm -f comp.bash

# The desktop board is a second binary, so the suite checks it was built and
# that it answers without opening a window — a headless runner can't do more
# than that, and this is what catches `amdui` disappearing from Cargo.toml.
# A CLI-only build (`cargo build -p amd-cli`) has no amdui, and that's not a
# failure.
echo "the desktop binary:"
AMDUI="${AMDUI_BIN:-${ROOT}/target/debug/amdui}"
if [ -x "${AMDUI}" ]; then
  assert "amdui --version names itself" \
    bash -c "'${AMDUI}' --version | grep -q '^amdui '"
  assert "amdui --help mentions the board" \
    bash -c "'${AMDUI}' --help | grep -q 'desktop board'"
  assert "amdui rejects an unknown argument" \
    bash -c "! '${AMDUI}' --nope"
else
  echo "  skip - no amdui binary (built without the gui feature)"
fi

# The terminal board is gone (ticket 082): no subcommand for it.
assert "amd has no tui subcommand" \
  bash -c "! '${AMD}' tui </dev/null >/dev/null 2>&1 && ! '${AMD}' --help | grep -q 'terminal board'"

echo "guards:"
assert "errors outside a git repository" \
  bash -c "cd '${nongit}' && ! '${AMD}' board"

echo "non-interactive (--no-input / AMD_NO_INPUT):"
assert "new with no title errors instead of prompting" \
  bash -c "! '${AMD}' new"
assert "new with no title shows the usage line" \
  bash -c "'${AMD}' new 2>&1 | grep -q 'usage: amd new'"
assert "amd todo with no ref lists instead of prompting, even non-interactively" \
  bash -c "'${AMD}' todo >/dev/null"
assert "--no-input works without the env var" \
  bash -c "env -u AMD_NO_INPUT '${AMD}' --no-input show 2>&1 | grep -q 'usage: amd show <ref>'"

echo "AMD_DIR:"
# A fresh repo, not `${tmp}`: README/settings now live at the repository
# root, so a second AMD_DIR-relocated board inside the same, already-heavily
# -used `${tmp}` repo would share (and inherit the counter from) the
# existing `tasks/` board's settings file — an edge case nobody hits in
# practice (one repo picks one board location), not what this checks.
dir_repo="$(mktemp -d)"
( cd "${dir_repo}" && git init -q && git config user.email t@t.co && git config user.name t )
assert "AMD_DIR relocates the board" \
  bash -c "cd '${dir_repo}' && AMD_DIR=work AMD_YES=1 '${AMD}' new 'Elsewhere' >/dev/null && test -f work/backlog/001-elsewhere.md"
# Left behind, this repo becomes the one board next to the next run's fresh
# repo in /tmp, and Board::discover() rightly finds it.
rm -rf "${dir_repo}"

echo "auto-create when no board exists:"
fresh="$(mktemp -d)"; ( cd "${fresh}" && git init -q )
assert "non-interactive with no board errors (no hang)" \
  bash -c "cd '${fresh}' && ! '${AMD}' board </dev/null"
assert "AMD_YES=1 creates the board and runs" \
  bash -c "cd '${fresh}' && AMD_YES=1 '${AMD}' board >/dev/null && test -d tasks/todo"
rm -rf "${fresh}"

echo
echo "passed: ${pass}, failed: ${fail}"
[ "${fail}" -eq 0 ]
