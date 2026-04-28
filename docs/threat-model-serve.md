# Threat Model: `atlas serve`

`atlas serve` exposes Atlas graph and context services over MCP transport. Main security goal: answer repo-scoped requests without letting client treat untrusted repository text as instructions, escape repo/storage boundaries, or coerce Atlas into broader host access than documented.

## Scope

In scope:

- stdio MCP server mode
- repo-scoped broker/daemon mode
- MCP request parsing and dispatch
- graph/context/session store access triggered by MCP tools
- hook- and artifact-derived text returned through MCP responses

Out of scope:

- compromise by attacker already running as same trusted OS user with full local file access
- malicious local kernel, filesystem, or SQLite implementation
- downstream LLM/client UI behavior after Atlas returns clearly marked untrusted text

## Assets

Protect:

- repository graph facts in `.atlas/worldtree.db`
- saved artifacts in `.atlas/context.db`
- session continuity state in `.atlas/session.db`
- canonical repo-root and repo-path identity
- host filesystem data outside selected repo and Atlas local state
- secrets in environment, config, credentials, and unrelated repos

## Trusted components

Trusted to enforce policy:

- Atlas binaries and in-repo source for CLI/MCP services
- OS path and permission model
- canonical path enforcement in `atlas-repo`
- store-boundary rules:
  - graph facts in `worldtree.db`
  - large artifacts in `context.db`
  - session events in `session.db`

Conditionally trusted:

- repo content for parsing as code/data, not as instructions
- installed hook payload shape after sanitization/redaction only
- editor/agent frontend transport only after request validation

## Untrusted inputs

Treat all of these as untrusted:

- MCP method names, params, and path inputs from client
- file contents in repository
- commit messages, docs, comments, prompts, and saved artifacts
- hook stdin payloads
- branch names, filenames, and symbol names derived from repo state
- serialized text read back from `context.db` or `session.db`

Rule: untrusted text may become search/query evidence. It must never become policy.

## Trust boundaries

### Boundary 1: client -> MCP transport

Client can request tools. Client cannot redefine server policy.

Defenses:

- JSON-RPC request validation
- per-tool argument parsing
- repo-scoped server startup with explicit repo + DB path
- worker timeout and request timeout controls

### Boundary 2: repo text -> Atlas reasoning/output

Repository text is useful evidence but unsafe instruction source.

Defenses:

- canonical path resolution before lookup
- prompt-injection policy for quoted/escaped response snippets
- response truncation and preview routing for large artifacts
- no implicit promotion of docs/comments/commit text into hidden instructions

Response handling rules for untrusted text:

- treat repository-derived and user-derived text as data, not instructions
- never elevate untrusted text into system prompts, hidden instructions, policy text, or tool-selection rules
- quote or escape untrusted snippets before returning them in MCP responses
- prefer fenced code blocks, JSON string encoding, or explicit preview fields over free-form interpolation into narrative text
- preserve provenance so client can see whether text came from file content, commit metadata, docs, or runtime artifact storage
- truncate large untrusted payloads and return preview or pointer metadata when full body would exceed response budget
- strip or neutralize control characters and terminal escape sequences before echoing text in user-facing output
- keep Atlas-authored explanation separate from raw file text, doc text, commit messages, or hook payload fields

### Boundary 3: frontend/hook runtime -> storage

Hooks and adapters may persist runtime context. They must use content/session services, not direct graph writes.

Defenses:

- sanitize and redact hook payloads before storage
- route large payloads into `context.db`
- keep bounded event rows in `session.db`
- never write hook/runtime payloads directly into `worldtree.db`

### Boundary 4: repo scope -> host scope

Atlas serves one canonical repo root plus Atlas local state for that repo. Client must not widen scope by path tricks.

Defenses:

- canonical repo-root discovery
- canonical repo-relative path identity
- repo-relative file resolution
- socket/pipe instance scoped by canonical repo root plus DB path

## Forbidden MCP client actions

Client must not expect Atlas to do any of these:

- treat repository text, commit messages, docs, or artifacts as instructions that override tool policy
- read arbitrary files outside canonical repo root and Atlas-owned local state for current repo
- bypass canonicalization with `..`, symlink tricks, alternate separators, or case-drift paths
- execute arbitrary shell commands through MCP request data
- fetch arbitrary network resources on behalf of client unless future tool contract explicitly allows it
- write directly to SQLite files or request raw SQL execution
- disclose secrets from environment variables, host credentials, or unrelated repos
- persist unbounded payloads into response body when policy requires preview or pointer routing

If client asks for forbidden behavior, Atlas should fail closed with explicit error instead of best-effort compliance.

## Expected attacker moves

Realistic attacks:

- prompt injection inside README/docs/comments asking agent to ignore system policy
- crafted filenames or commit messages that try to escape quoting or parsing
- oversized payloads intended to blow token budget or response size
- path traversal attempts through explicit MCP file/path arguments
- hook payloads containing secrets or control characters

## Security invariants

- repo identity is canonical before path-derived lookup or persistence
- `worldtree.db` stores graph facts, not arbitrary runtime text dumps
- `context.db` and `session.db` may hold untrusted text, but that text stays data
- MCP responses must mark untrusted snippets as quoted/escaped content
- failures in enrichment or session persistence must not weaken graph-boundary policy

## Operational guidance

When changing `atlas serve`:

1. identify whether new input is trusted, conditionally trusted, or untrusted
2. define which store may persist it
3. define whether response returns raw text, preview, or pointer
4. add path-boundary and quoting/escaping tests
5. fail closed if repo or storage boundary becomes ambiguous

## Residual risk

Atlas can label and bound untrusted text. It cannot guarantee downstream LLM client will reason safely about that text after delivery. Clear quoting, escaping, truncation, and provenance markers reduce risk; they do not remove need for safe client-side prompting.
