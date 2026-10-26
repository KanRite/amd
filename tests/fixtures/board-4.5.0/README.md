# A board written by agile-md 4.5.0

`tests/test.sh` copies this into a fresh repository and checks that the
current build reads and changes it (ticket 193: every release reads what an
earlier one wrote). Made on 2026-10-19 with the 4.5.0 release's `rite`,
nothing edited by hand:

```bash
rite init fixture --local-only
rite tags add CLI && rite tags add BUG
rite group epic checkout && rite group sprint s1 --days 10
rite new "Plain ticket"
rite new "A bug in the basket" --tag BUG --branch-type bugfix
rite new "Due and repeating" --due 2030-01-15 --recur monthly
rite new "Needs the bug fixed" --requires 2 --tag CLI
rite new "An idea" --draft
rite new "Child of the plain one" --parent 1
rite new "Filed in the epic" && rite new "In the sprint"
rite set 1 priority medium && rite set 2 points 3 && rite set 8 points 2
rite checkout 7 && rite s1 8
rite todo 2 && rite doing 2 && rite todo 1
rite todo 3 && rite doing 3 && rite done 3   # makes 009, the next one
```

Don't regenerate it with a newer release: its point is to be old. Add a
fixture for a later release beside it instead.
