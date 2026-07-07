# Atlas Error Codes

Canonical catalog for user-facing Atlas error codes.

## MCP Error Handling Contract

Atlas expose MCP failures in two layers:

- **protocol error**: top-level JSON-RPC `error`; request failed before valid tool execution
- **tool execution error**: JSON-RPC `result` with `isError: true`; request reached tool dispatch and failed after that boundary

Machine-readable fields:

- protocol error => `error.data.atlas_error_code`
- tool execution error => `structuredContent.code`

Human-readable field:

- tool execution error => `content[0].text`

Clients should classify from machine-readable code fields, not by parsing free text.

Atlas surfaces these codes in two places:

- `error_code` in CLI JSON and MCP tool JSON payloads
- `atlas_error_code` in MCP JSON-RPC transport errors and compact wrapper metadata

MCP responses that carry one of these codes should link back here through `error_code_docs` or `atlas_error_code_docs`.

## Graph And Tool Status Codes

<a id="none"></a>
### `none`

No error. Graph or tool state is healthy for requested operation.

<a id="missing_graph_db"></a>
### `missing_graph_db`

Graph database file is missing.

<a id="noncanonical_path_rows"></a>
### `noncanonical_path_rows`

Persisted rows use non-canonical repo paths and require rebuild or cleanup.

<a id="schema_mismatch"></a>
### `schema_mismatch`

Stored SQLite schema does not match current Atlas build.

<a id="corrupt_or_inconsistent_graph_rows"></a>
### `corrupt_or_inconsistent_graph_rows`

Graph database opened, but integrity or logical consistency checks failed.

<a id="interrupted_build"></a>
### `interrupted_build`

Previous build was left in in-progress state.

<a id="degraded_build"></a>
### `degraded_build`

Build completed with degraded or budget-limited state.

<a id="failed_build"></a>
### `failed_build`

Last build failed.

<a id="stale_index"></a>
### `stale_index`

Indexed graph is older than current graph-relevant repo changes.

<a id="retrieval_index_unavailable"></a>
### `retrieval_index_unavailable`

Retrieval/content index is unavailable or not searchable.

<a id="node_not_found"></a>
### `node_not_found`

Requested graph symbol or node could not be resolved.

<a id="checks_failed"></a>
### `checks_failed`

One or more doctor or validation checks failed.

<a id="unknown_stage"></a>
### `unknown_stage`

Requested postprocess stage name is not supported.

<a id="selfupdate_not_supported"></a>
### `selfupdate_not_supported`

Self-update command is unavailable in current Atlas build and install flow.

## MCP Transport Codes

<a id="parse_error"></a>
### `parse_error`

Incoming JSON-RPC message could not be parsed.

<a id="invalid_request"></a>
### `invalid_request`

Incoming JSON-RPC envelope is structurally invalid.

<a id="method_not_found"></a>
### `method_not_found`

Requested JSON-RPC or MCP method name is unknown.

<a id="invalid_params"></a>
### `invalid_params`

Method arguments failed validation.

<a id="internal_error"></a>
### `internal_error`

Server hit an internal failure before producing tool result.

<a id="tool_execution_failed"></a>
### `tool_execution_failed`

Tool dispatch started but failed during execution.

## Tool Execution Error Codes

These codes appear in MCP tool `structuredContent.code` and CLI JSON `error_code`.

| code | meaning | retryability | stable `details` keys |
|------|---------|--------------|------------------------|
| `invalid_input` | business-valid request shape reached tool, but handler-specific validation failed | retry after fixing input | `detail`, sometimes `path` |
| `file_not_found` | requested repo-relative file does not exist | retry after fixing path or creating file | `detail`, `path` |
| `symbol_not_found` | requested symbol or qualified name could not be resolved by tool | retry after fixing symbol or refreshing graph | `detail`, `qualified_name` when available |
| `graph_stale` | graph readiness blocked execution or handler detected stale graph state | retry after `build_or_update_graph` or with allowed stale mode when supported | `detail`, `execution_state`, `reason`, `suggestions`, `pending_change_count` when available |
| `timeout` | tool timed out after dispatch started | retry with narrower scope, higher timeout, or healthier dependency | `detail`, `timeout_ms`, `request_id`, `method` |
| `dependency_failed` | downstream dependency or service failed | retry after dependency recovers | `detail`, `service`, `status` when available |
| `internal_tool_error` | unexpected internal failure happened inside tool execution boundary | retry only after server state or code issue is fixed | `detail` |

Notes:

- `detail` is stable as key name, not stable in exact text.
- listed keys are stable when present; tools may omit keys that do not apply.
- `content[0].text` should stay concise and display-oriented; parse `structuredContent` instead.

<a id="worker_unavailable"></a>
### `worker_unavailable`

No worker was available to run requested MCP tool.

<a id="request_timed_out"></a>
### `request_timed_out`

Tool request exceeded configured timeout.

<a id="rate_limited"></a>
### `rate_limited`

Server rejected request because rate limit was exceeded.
