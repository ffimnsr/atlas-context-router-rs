# Changelog

All notable changes to this project should be recorded in this file.

Versioning policy may evolve while Atlas is still moving quickly, but release notes should still group changes by user-visible impact.

## 1.5.1 - 2026-07-22

### Features

- add tool searches and tool list on mcp (`7dbd170`)
- lay the foundation for a stable mcp tool output (`17b622b`)
- stabilize mcp tools part 2 (`16bc2a2`)
- update mcp tools stability part 3 (`b0bf35f`)
- do the 5th mcp tool stabilization (`836e05b`)
- update tools stability part 5 (`425bee2`)
- cleanup on tool response fields (`d208514`)


## 1.5.0 - 2026-07-10

### Features

- update and harden some tools (`5cf327f`)
- refactor the transport.rs to module (`569f89c`)
- refactor discovery tools from monolithic to modular (`91b0c28`)
- use dynamic root captured from client root list (`39545a4`)
- update the tests path canonicalization and fix lint errors (`86b44f6`)
- add cwd fallback if there's no dynamic root (`1e03c82`)
- add man tools (`9f4dccc`)


### Documentation

- add new feature man tool for help docs on mcp (`f0e99e5`)
- update new issues.md (`a5f9317`)


## 1.4.2 - 2026-07-07

### Features

- add correct error response (`49fdff7`)


## 1.4.1 - 2026-07-07

### Features

- add roots/list instead of repo flag on serve (`919f53e`)


## 1.4.0 - 2026-07-07

### Features

- add bench and proptests and fix some bugs (`a85c6e9`)
- fix ci errors and benches (`50ea37b`)
- update the serve tool to have new direct stdio flag (`114b851`)
- add new issues for mcp spec (`31c5686`)
- implement MCP spec first phase (`bd4f8ce`)
- add changes for second and third phase of MCP upgrade (`5ed91e7`)
- add new tools and completions for the new spec (`dd6b13b`)
- implement the phase 5 mcp changes (`8771a38`)
- add all gates test (`5ebb6bb`)
- complete the phase 7 of MCP upgrade implementation (`da17b2c`)
- add fixes for mcp upgrade (`8ef8b4d`)


### CI

- update bench for git init and ci workflow (`f42a277`)


## 1.3.0 - 2026-05-11

### Features

- update history build/update progress bar for saving to store (`71e3a3f`)
- update history build and update to have proper estimate with recompute (`2abd0a8`)
- add new item section for insights (`1efd702`)
- add insights complete phase (`65baa30`)
- update rust parser to use tree-sitter scm (`25c68ab`)
- update golden as now it emits proper on rust (`c7a49da`)
- update dead code analysis for code only (`f74a81a`)
- fix fuzz crashes on invariant (`16c85b4`)


### CI

- add fuzzing on workflow (`129d497`)
- update fuzz workflow (`36a883b`)


### Maintenance

- update rustfmt (`7ed14ae`)


## 1.2.2 - 2026-05-07

### Features

- add preflight estimates and confirmation for history operations (`41d1dcb`)


### Fixes

- error on CI bug (`e13719a`)


## 1.2.1 - 2026-05-06

### Features

- query explanation & retrieval mode diagnostics (`aad1eb0`)
- ship ranking evidence, docs/postprocess/readiness patches; add embedding dimension freeze and backend capabilities (`9e80471`)
- implement graph/content companion contract with unified ranking (`1f02a86`)


## 1.2.0 - 2026-05-06

### Features

- move embedding configuration from env vars to .atlas/config.toml (`d4ffe64`)
- implement canonical graph readiness source-of-truth (Patch S) (`f8a2f01`)


## 1.1.2 - 2026-05-05

### Features

- add docs section discovery surfaces (`3755be5`)
- expand fuzz harness coverage and add new fuzz targets (`4f2490d`)


### Maintenance

- move shell completions installation to install.sh (`8e9a6f3`)


## 1.1.1 - 2026-04-29

### Features

- daemon crash recovery with exponential backoff and panic handling (`5eb9012`)


### Fixes

- serialize embed env-var tests to prevent race condition (`f3da333`)


### Maintenance

- consolidate summary output formatting (`48428bb`)


## 1.1.0 - 2026-04-28

### Features

- embed integration and schema generation (`1052a75`)


### Fixes

- reset accepted socket to blocking mode on macOS (`9fa30ae`)
- canonicalize tempdir in path test for macOS /var symlink (`b97b298`)


### Maintenance

- update cli, engine, and supporting infrastructure (`d1ae6a9`)
- documentation, tests, and infrastructure updates (`d6e1183`)


## 1.0.2 - 2026-04-28

### Features

- update wiki reference (`a9ef2a2`)
- implement agent-scoped context and session management (`9266ed3`)


### Documentation

- move Phase CM14 and CM15 to SHIPPED (`34dec7f`)


### Maintenance

- rename 'Repo root' to 'Atlas scope', add git root detection (`b539f93`)
- update release script (`015aafa`)
