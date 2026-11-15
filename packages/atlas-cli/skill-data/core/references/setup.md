# Setup, serve, and health

## Install integration

```bash
atlas install                                  # git hooks, hook runner, AGENTS.md block, skill stub
atlas install --instructions-only              # managed AGENTS.md block only
atlas install --validate-only                  # report current state, write nothing
atlas install --dry-run                        # preview every write
atlas install --mode cli                       # AGENTS.md block and skill stub only
atlas install --mode hook                      # git hooks and hook runner only
atlas install --no-hooks --no-instructions     # explicit skips
atlas install --force                          # also refresh user-edited skill stub and non-Atlas hooks
```

`atlas install` writes only:

- the managed `AGENTS.md` block — a stable pointer to `atlas skills get core`
- `.agents/skills/atlas/SKILL.md` — discovery stub, created when missing
- `.git/hooks/{pre-commit,post-commit,post-checkout,post-merge,post-rewrite}`
- `.atlas/hooks/atlas-hook`

It never writes per-tool MCP or hook configuration files.

## Init modes

```bash
atlas init                       # generic MCP registration asset (default mode: mcp)
atlas init --mode cli            # managed AGENTS.md block and skill stub
atlas init --mode hook           # git hooks and the hook runner
atlas init --mode all
atlas init --profile standard
```

## Agent skills

```bash
atlas skills list
atlas skills get core
atlas skills get core --ref workflows
atlas skills get core --full
```

Guides are embedded in the installed binary, so they always match the installed version. `--ref` prints one reference; `--full` prints the guide plus every reference.

## Build and refresh

```bash
atlas build
atlas update --working-tree
atlas update --base origin/main
atlas update --staged
atlas update --files src/lib.rs src/session.rs
```

Add `--dry-run` to preview, `--fail-fast` to stop on the first parse error. Multi-repo workspaces add `--repo-id <ID>`, `--all-repos`, or `--affected-repos`.

## MCP server

Register Atlas as a stdio MCP server:

```text
command: atlas
args:    ["serve"]                                  # global
args:    ["--repo", "/abs/path/to/repo", "serve"]   # pin one repository
```

```json
{
  "mcpServers": {
    "atlas": { "type": "stdio", "command": "atlas", "args": ["serve"] }
  }
}
```

`atlas serve --direct-stdio` bypasses the repo-scoped broker. `atlas serve-http` starts the Streamable HTTP transport when the binary is built with `--features http-transport`.

## Health and configuration

```bash
atlas status
atlas doctor
atlas db-check
atlas config show
atlas debug-graph
atlas debug-config
```

Start with `status` (or the `status` MCP tool) when graph health is unknown, and with `doctor` when the repo, config, or database looks wrong. `db-check` covers SQLite integrity plus orphan and dangling rows.
