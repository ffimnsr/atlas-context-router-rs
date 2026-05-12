# Atlas Error Codes

Canonical catalog for user-facing Atlas error codes.

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

<a id="worker_unavailable"></a>
### `worker_unavailable`

No worker was available to run requested MCP tool.

<a id="request_timed_out"></a>
### `request_timed_out`

Tool request exceeded configured timeout.

<a id="rate_limited"></a>
### `rate_limited`

Server rejected request because rate limit was exceeded.
