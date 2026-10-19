# A board written by agile-md 4.5.0

`tests/test.sh` copies this into a fresh repository and checks that the
current build reads and changes it (ticket 193: every release reads what an
earlier one wrote). Made on 2026-10-19 with the 4.5.0 release's `amd`,
nothing edited by hand:

```bash
amd init fixture --local-only
amd tags add CLI && amd tags add BUG
amd group epic checkout && amd group sprint s1 --days 10
amd new "Plain ticket"
amd new "A bug in the basket" --tag BUG --branch-type bugfix
amd new "Due and repeating" --due 2030-01-15 --recur monthly
amd new "Needs the bug fixed" --requires 2 --tag CLI
amd new "An idea" --draft
amd new "Child of the plain one" --parent 1
amd new "Filed in the epic" && amd new "In the sprint"
amd set 1 priority medium && amd set 2 points 3 && amd set 8 points 2
amd checkout 7 && amd s1 8
amd todo 2 && amd doing 2 && amd todo 1
amd todo 3 && amd doing 3 && amd done 3   # makes 009, the next one
```

Don't regenerate it with a newer release: its point is to be old. Add a
fixture for a later release beside it instead.
