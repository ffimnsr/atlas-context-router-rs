# Support

## Where to ask

- Bugs: open a GitHub issue.
- Feature requests: open a GitHub issue with expected workflow and constraints.
- Usage questions: open a GitHub issue and include command, flags, and repo context.
- Security concerns: follow [SECURITY.md](SECURITY.md); do not post exploit details publicly.

## What to include

- Atlas version or commit
- OS and shell
- exact command run
- expected behavior
- actual behavior
- logs or error output
- whether `.atlas/` state was built fresh or reused

## Self-check before opening an issue

- read [README.md](README.md)
- search existing issues
- run `cargo test --workspace`
- run `cargo clippy --workspace --all-targets --all-features -- -D warnings` if change is local development related
