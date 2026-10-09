#!/usr/bin/env bash
# Builds a demo project for screenshots (ticket 147): a fictional online shop,
# "Acme", with a board that shows everything amd does — every column, an epic,
# a running sprint, assignees, priorities, points, tags, a draft, locks,
# related tickets, two code repositories, and documents (the project's own
# and a shared handbook). Every name is made up and every
# address is @example.com, so a screenshot shows nobody's real data.
#
#   scripts/demo-board.sh [DIR]      # default: ./agile-md-demo
#
# It keeps its own agile-md config in DIR/config, so your registry, theme and
# remembered board are untouched. Open it with the command it prints at the
# end. Run it again to rebuild from scratch (it refuses a DIR it didn't make).
set -euo pipefail

DIR="${1:-./agile-md-demo}"
SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Which amd: $AMD if given, else the installed one, else a build in this
# clone. Never a stale build over one you asked for.
if [ -z "${AMD:-}" ]; then
  AMD="$(command -v amd || true)"
  [ -z "${AMD}" ] && [ -x "${SELF}/target/release/amd" ] && AMD="${SELF}/target/release/amd"
  [ -z "${AMD}" ] && [ -x "${SELF}/target/debug/amd" ] && AMD="${SELF}/target/debug/amd"
fi
[ -n "${AMD}" ] || { echo "amd not found: install it, or build it here first" >&2; exit 1; }

if [ -e "${DIR}" ]; then
  [ -f "${DIR}/.agile-md-demo" ] || { echo "${DIR} exists and isn't a demo board; choose another DIR" >&2; exit 1; }
  rm -rf "${DIR}"
fi
mkdir -p "${DIR}"
DIR="$(cd "${DIR}" && pwd)"
touch "${DIR}/.agile-md-demo"

export XDG_CONFIG_HOME="${DIR}/config" AMD_NO_INPUT=1 AMD_NO_BRANCH=1 AMD_SYNC=0

# The tickets repository and two code repositories, all committed as Alex.
repo() {
  git init -q -b main "${DIR}/$1"
  git -C "${DIR}/$1" config user.name "Alex Morgan"
  git -C "${DIR}/$1" config user.email "alex.morgan@example.com"
  git -C "${DIR}/$1" config commit.gpgsign false
}
for r in acme-tickets acme-web acme-api; do repo "${r}"; done
git -C "${DIR}/acme-web" commit -q --allow-empty -m "Storefront"
git -C "${DIR}/acme-api" commit -q --allow-empty -m "Orders API"

cd "${DIR}/acme-tickets"
amd() { "${AMD}" "$@" >/dev/null 2>&1; }
# Records what's staged, if anything: amd has usually done it already.
save() { git diff --cached --quiet || git commit -q -m "$1"; }
amd init acme --local-only
git add -A && save "Board"
amd register ../acme-web
amd register ../acme-api

# The team, and the project's tags.
sed -i.bak '/^users *=/d' .settings && rm -f .settings.bak
echo "admins = Alex Morgan <alex.morgan@example.com>" >> .settings
echo "users = Alex Morgan <alex.morgan@example.com>, Priya Shah <priya.shah@example.com>, Sam Lee <sam.lee@example.com>, Jordan Reyes <jordan.reyes@example.com>" >> .settings
for tag in BUG FEATURE UX API SECURITY PERFORMANCE DOCS MOBILE PAYMENTS ACCESSIBILITY; do amd tags add "${tag}"; done
git add .settings && save "Team and tags"

# new TITLE [amd new options...] — prints the new ticket's number.
new() {
  "${AMD}" new "$@" --no-edit 2>/dev/null | sed -n 's#^created backlog/\([0-9]*\)-.*#\1#p'
}
# note ID TEXT — puts TEXT under the ticket's Notes heading.
note() {
  local file
  file="$(find tasks -name "$1-*.md" -not -path '*/archive/*' | head -1)"
  awk -v text="$2" '{print} /^## Notes$/ && !done {print ""; print text; done=1}' "${file}" > "${file}.tmp"
  mv "${file}.tmp" "${file}"
  git add "${file}" && save "notes: $1"
}
size() { amd set "$1" points "$2"; }
prio() { amd set "$1" priority "$2"; }
walk() { local id="$1"; shift; for column in "$@"; do amd "${column}" "${id}"; done; }

# Epics and a sprint.
amd group epic checkout --description "A faster, simpler checkout"
amd group epic onboarding --description "First-run experience for new customers"
amd group sprint s12 --days 14

# The sprint: sized, assigned, started.
s1=$(new "Apple Pay at checkout" --branch-type feature -R acme-web -t PAYMENTS -t FEATURE -a "Priya Shah")
s2=$(new "Retry failed card payments" --branch-type feature -R acme-api -t PAYMENTS -t API -a "Sam Lee")
s3=$(new "Order confirmation email shows the wrong total" --branch-type bugfix -R acme-api -t BUG -a "Jordan Reyes")
s4=$(new "Save the basket between visits" --branch-type feature -R acme-web -t FEATURE -t UX)
for id in "${s1}" "${s2}" "${s3}" "${s4}"; do size "${id}" 5; done
size "${s3}" 2; size "${s4}" 3
prio "${s1}" high; prio "${s3}" high; prio "${s2}" medium
for id in "${s1}" "${s2}" "${s3}" "${s4}"; do amd s12 "${id}"; done
amd group start s12

# The checkout epic.
e1=$(new "One-page checkout" --branch-type feature -R acme-web -t UX -t FEATURE -a "Priya Shah")
e2=$(new "Show delivery dates before payment" --branch-type feature -R acme-web -t UX -p "${e1}")
e3=$(new "Address lookup by postcode" --branch-type feature -R acme-api -t API -p "${e1}" -r "${e2}")
size "${e1}" 8; size "${e2}" 3; size "${e3}" 5; prio "${e1}" medium
for id in "${e1}" "${e2}" "${e3}"; do amd checkout "${id}"; done

# The onboarding epic.
o1=$(new "Welcome tour for first-time customers" --branch-type feature -R acme-web -t UX -t MOBILE)
o2=$(new "Explain delivery options on sign-up" -t DOCS -t UX)
for id in "${o1}" "${o2}"; do amd onboarding "${id}"; done

# The loose backlog, a draft among them.
new "Search results ignore spelling mistakes" --branch-type feature -R acme-api -t FEATURE -t API >/dev/null
new "Dark mode for the storefront" --branch-type feature -R acme-web -t UX -t ACCESSIBILITY >/dev/null
b3=$(new "Rate limit the login endpoint" --branch-type hotfix -R acme-api -t SECURITY -t API -a "Sam Lee")
b4=$(new "Gift cards" --draft -t PAYMENTS -t FEATURE)
b5=$(new "Product images load slowly on mobile" --branch-type bugfix -R acme-web -t PERFORMANCE -t MOBILE -t BUG)
prio "${b3}" high; prio "${b5}" medium; size "${b3}" 2; size "${b5}" 3
note "${b4}" "Still being shaped: physical cards, digital codes, or both? Check with finance before sizing."

# Work in progress: todo, doing (at its WIP limit), done.
t1=$(new "Screen reader labels on the product page" --branch-type bugfix -R acme-web -t ACCESSIBILITY -t BUG -a "Jordan Reyes")
t2=$(new "Cache the product catalogue" --branch-type feature -R acme-api -t PERFORMANCE -t API -a "Sam Lee")
d1=$(new "Fix the basket total rounding" --branch-type bugfix -R acme-api -t BUG -t PAYMENTS -a "Alex Morgan")
d2=$(new "Write the returns policy page" -t DOCS -a "Alex Morgan")
f1=$(new "Upgrade the payment library" --branch-type chore -R acme-api -t SECURITY -t PAYMENTS -a "Sam Lee")
f2=$(new "Mobile menu overlaps the logo" --branch-type bugfix -R acme-web -t BUG -t MOBILE -a "Priya Shah")
f3=$(new "Publish the API rate limits" -t DOCS -t API -a "Jordan Reyes")
prio "${t1}" high; prio "${d1}" high; prio "${t2}" medium
size "${t1}" 2; size "${t2}" 5; size "${d1}" 3; size "${d2}" 1
amd link "${d1}" "${s3}"
note "${d1}" "Totals are summed from rounded line prices; round once, at the end. Same root cause as the confirmation email (see related)."
# Done first: doing has a WIP limit of 2, and the finished work passes
# through it. Entering doing locks a ticket for whoever moved it (Alex,
# here), and leaving it unlocks it again.
walk "${f1}" todo doing "done"; walk "${f2}" todo doing "done"; walk "${f3}" todo doing "done"
walk "${d1}" todo doing; walk "${d2}" todo doing
walk "${t1}" todo; walk "${t2}" todo

# Spread the creation dates over the last few weeks, so "oldest" means something.
day=0
for f in tasks/*/[0-9]*.md tasks/backlog/*/[0-9]*.md; do
  [ -f "${f}" ] || continue
  day=$(( (day + 3) % 40 ))
  date="$(date -d "-$((day + 2)) days" +%F 2>/dev/null || date -v-"$((day + 2))"d +%F)"
  sed -i.bak "s/^created: .*/created: \"${date}\"/" "${f}" && rm -f "${f}.bak"
done
git add -A tasks && save "Spread creation dates"

# Documents (ticket 129): the project's own, and a handbook every project
# shares, named in the project's settings.
for r in acme-documents acme-handbook; do repo "${r}"; done
mkdir -p "${DIR}/acme-documents/adr" "${DIR}/acme-documents/rfc" "${DIR}/acme-documents/docs/img" "${DIR}/acme-handbook"
cat > "${DIR}/acme-documents/README.md" <<'MD'
# Acme engineering

How the Acme shop is built, and why. Decisions are in [adr/](adr/0001-one-page-checkout.md),
proposals in [rfc/](rfc/0001-payment-retries.md), and how-tos in [docs/](docs/releasing.md).

- [x] Storefront on the new checkout
- [ ] Payment retries
- [ ] Gift cards (being shaped)
MD
cat > "${DIR}/acme-documents/adr/0001-one-page-checkout.md" <<'MD'
# ADR 0001: One-page checkout

**Status:** accepted

## Context

Customers drop out between the address and payment steps. Each step is a page load on a slow phone.

## Decision

Checkout becomes one page: basket, address, delivery and payment, each a section that opens in turn.

## Consequences

- Fewer page loads, and the total stays in view.
- Address lookup ([RFC 0001](../rfc/0001-payment-retries.md) depends on it) moves into the API.
MD
cat > "${DIR}/acme-documents/rfc/0001-payment-retries.md" <<'MD'
# RFC 0001: Retry failed card payments

A card payment that fails for a temporary reason is retried, so the customer isn't asked twice.

| Decline code | Retry? | After |
|---|---|---|
| `insufficient_funds` | no | — |
| `processing_error` | yes | 30 s |
| `issuer_unavailable` | yes | 2 min |

```text
attempt 1 ──fail──▶ wait ──▶ attempt 2 ──fail──▶ tell the customer
```

> Never retry a payment the customer cancelled.
MD
cat > "${DIR}/acme-documents/docs/img/release.svg" <<'SVG'
<svg xmlns="http://www.w3.org/2000/svg" width="420" height="70" font-family="sans-serif" font-size="14">
  <rect x="5" y="15" width="110" height="40" rx="6" fill="#e0e7ff" stroke="#4f46e5"/><text x="60" y="40" text-anchor="middle">merge</text>
  <rect x="155" y="15" width="110" height="40" rx="6" fill="#e0e7ff" stroke="#4f46e5"/><text x="210" y="40" text-anchor="middle">tag vX.Y.Z</text>
  <rect x="305" y="15" width="110" height="40" rx="6" fill="#dcfce7" stroke="#16a34a"/><text x="360" y="40" text-anchor="middle">release</text>
  <path d="M115 35h40M265 35h40" stroke="#334155" stroke-width="2"/>
</svg>
SVG
cat > "${DIR}/acme-documents/docs/releasing.md" <<'MD'
# Releasing

![How a release happens](img/release.svg)

1. Merge to `main`.
2. Tag it: `git tag -a v1.4.0`.
3. The release workflow builds and publishes it.

See the [handbook](https://example.com/handbook) for the review rules.
MD
cat > "${DIR}/acme-handbook/README.md" <<'MD'
# The Acme handbook

Shared by every Acme project.

## Code review

- Every change goes through a pull request.
- One approval; two for anything touching payments.
MD
for r in acme-documents acme-handbook; do git -C "${DIR}/${r}" add -A && git -C "${DIR}/${r}" commit -q -m "Documents"; done
amd shared-documents acme-handbook

cat <<EOF
Demo board ready: ${DIR}
  desktop board:  cd "${DIR}" && XDG_CONFIG_HOME="${DIR}/config" amdui
  command line:   cd "${DIR}/acme-tickets" && XDG_CONFIG_HOME="${DIR}/config" amd board
Rebuild any time:  $0 "${DIR}"
EOF
