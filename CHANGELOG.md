# Changelog

All notable changes to this project should be recorded in this file.

Versioning policy may evolve while Atlas is still moving quickly, but release notes should still group changes by user-visible impact.

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
