# Security policy

## Supported versions

Fixes go into the latest 4.x release. Older releases aren't patched: the way
to a fix is to update, which the installer does when run again:

```bash
curl -fsSL https://raw.githubusercontent.com/kanrite/amd/v4/install.sh | bash
```

## Reporting a vulnerability

Please report it **privately**, not in a public issue: use
[Report a vulnerability](https://github.com/kanrite/amd/security/advisories/new)
on the repository's Security tab. Say what you found, how to reproduce it,
and which version (`amd --version`) and system you saw it on.

What happens next:

- We acknowledge your report **within 7 days**.
- We look into it and keep you informed in the advisory as we go.
- When it's fixed, we release the fix and publish a security advisory,
  crediting you if you'd like to be credited.

Please give us a chance to release a fix before telling anyone else about it.

## What's covered

- `amd`, the command line, including `amd mcp`, the server AI tools connect
  to.
- `amdui`, the desktop board.
- `install.sh` and the uninstaller it writes, `amd-uninstall`.
- The release builds on this repository's Releases page.

agile-md runs on your own machine, has no server and holds no data of its
own: your tickets, documents and ideas are files in your git repositories.
So these aren't covered here:

- The contents of your repositories, and who can read or change them (that's
  your git host's settings, such as GitHub's access and branch protection).
- Third-party tools: git, the `gh` CLI, your AI tool, WebKitGTK and the
  like. Report those to their own projects.
