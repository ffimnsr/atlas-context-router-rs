# Contributing

## Scope

Atlas focuses on this core chain:

- repo scan
- parse
- persist graph
- update incrementally
- search and traverse
- build review context

Optional features should not delay that core path.

## Before opening work

- Search [ISSUES.md](ISSUES.md) for existing roadmap item or follow-up patch.
- Prefer smallest safe change that fixes root cause.
- Reuse existing helpers before adding new abstractions.
- Keep diffs focused. Avoid unrelated refactors.

## Setup

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Coding rules

- Follow `rustfmt.toml` and `.editorconfig`.
- Use canonical repo paths for path-derived identity.
- Keep graph storage, content storage, and session storage separate.
- Prefer `Result` plus context over ignored errors.
- Add tests for new behavior and bug fixes.
- Do not preserve deprecated compatibility paths unless task explicitly requires it.

## Pull requests

- Link relevant issue, roadmap section, or patch section from [ISSUES.md](ISSUES.md).
- Explain user-visible behavior change and any migration or compatibility impact.
- Include tests or explain why no test is needed.
- Keep PR title and commit subjects in conventional-commit style when practical: `feat: ...`, `fix: ...`, `chore: ...`.

## Review expectations

Review prioritizes:

- correctness
- behavior regressions
- graph/readiness safety
- test coverage
- storage boundary violations

## Questions and support

General usage questions, bug reports, and feature requests belong in GitHub Issues. Sensitive security reports belong in the process described in [SECURITY.md](SECURITY.md).
