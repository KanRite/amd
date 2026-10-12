# Contributing to agile-md

Thanks for taking an interest. agile-md is MIT licensed; anything you
contribute is under the same licence.

## Reporting a bug or asking for something

Open an [issue](https://github.com/agile-md/amd/issues/new/choose). There's
a form for bugs and one for ideas.

The project's own board is agile-md itself: open issues are pulled into the
maintainers' backlog automatically, tagged `TRIAGE`, and looked at from
there. Once one becomes work, the issue is where you'll hear about it.

For a security problem, don't open a public issue; see "Security" below.

## Changing the code

1. Fork the repository and branch from `main`. Branch names start with a
   type: `feature/`, `bugfix/`, `hotfix/`, `release/` or `chore/`
   (for example `bugfix/drag-onto-a-full-column`).
2. Make the change. [README.md](README.md#development) has the build and
   the checks; CI runs all of them, so run them first:

   ```bash
   (cd crates/amd-ui-web && trunk build --release)   # before anything builds amdui
   cargo test --workspace --all-features
   bash tests/test.sh
   cargo clippy --all-targets --all-features -- -D warnings
   cargo clippy -p amd-ui-web -p amd-ui-model --target wasm32-unknown-unknown -- -D warnings
   cargo fmt --all --check
   ```

   Building amdui on Linux needs WebKitGTK and ALSA's headers
   (`libwebkit2gtk-4.1-dev`, `libasound2-dev`), plus Trunk and the
   `wasm32-unknown-unknown` target.
3. Commit with a conventional type: `feat`, `fix`, `docs`, `style`,
   `refactor`, `perf`, `test`, `build`, `ci`, `chore` or `revert`
   (`fix: a drag onto a full column asks first`).
4. Open a pull request against `main` and say which issue it's for. A
   maintainer reviews it; `main` needs one approval and green checks.

Some things worth knowing first:

- **`tests/test.sh` is the specification.** A change the command line can
  see gets an assertion there, not only a Rust unit test, and the
  command's `--help` text should still be true.
- **Ticket logic lives in `amd-lib`.** The command line and the desktop
  board both call it; neither has rules of its own.
- **The ticket file format is a public contract.** Templates and the
  `TaskContext` variables are used by other people's boards, so adding is
  fine and renaming breaks them.
- [CLAUDE.md](CLAUDE.md) describes the architecture in detail.

## Maintainers

The maintainers' tickets (`agile-md-tickets`) and design documents
(`agile-md-documents`) are private repositories in the
[agile-md](https://github.com/agile-md) organisation. To work from them,
become a maintainer: you're added to the organisation, and then
`amd join agile-md` clones both and adds you to the project's users.
Everyone else works from issues and pull requests here.

## Security

Please report a vulnerability privately rather than in an issue: use
[Report a vulnerability](https://github.com/agile-md/amd/security/advisories/new)
on the repository's Security tab.

## Conduct

Everyone taking part is expected to follow the
[code of conduct](CODE_OF_CONDUCT.md).
