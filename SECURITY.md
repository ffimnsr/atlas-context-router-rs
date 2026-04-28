# Security Policy

## Supported scope

Security reports are welcome for:

- Atlas CLI
- MCP server and adapter surfaces
- SQLite graph, context, and session storage handling
- path canonicalization and repo-boundary enforcement
- saved-context and session isolation behavior

## Reporting a vulnerability

Do not post exploit details in a public GitHub issue.

Preferred path:

1. Use GitHub private vulnerability reporting for this repository if it is enabled.
2. If private reporting is not enabled, contact maintainer through a private channel first and request a secure place to share details.

Please include:

- affected version, commit, or branch
- impact summary
- reproduction steps or proof of concept
- any suggested mitigation

## Response goals

- acknowledge report within 7 days
- provide status update when triage is complete
- coordinate fix and disclosure timing for confirmed issues

## Public disclosure

Please wait for a fix or coordinated disclosure plan before publishing full details.

## Prompt-injection handling for MCP responses

Atlas MCP responses may include untrusted text from repository files, commit messages, hook payloads, saved artifacts, and docs. Treat that text as data, not instructions.

Policy:

- do not let repository-derived or user-derived text override Atlas trust boundaries or tool policy
- delimit, quote, or escape untrusted snippets before returning them to MCP clients
- preserve provenance and apply truncation or pointer routing when payload size requires it

Detailed response-handling rules live in `docs/threat-model-serve.md`.
