# Atlas — Stateful Coding Agent Backend

Instruction for all the items in this file:
- Keep each checklist item scoped to one small workable chunk.
- Describe exact code, command, schema field, validation rule, or test to add/change.
- Do not combine multiple implementation steps into one checklist item if they can be merged separately.
- Prefer additive wording like "add", "replace", "update", "remove", "validate", "test".
- Avoid broad goals without concrete implementation detail.

## Goal

Create a cli for stateful coding agent backend.

The primary behavior to preserve is:

- build a repository code graph
- incrementally update it from git diffs
- persist graph data in SQLite
- query graph structure and impact radius
- assemble review context from changed files and neighboring nodes
- expose a CLI first, with MCP later
- make sure to ALWAYS sync CLI tool and MCP tooling (including its flags)

For terms that are easy to misread in this document:

- `flow`: named ordered path or scenario over existing graph nodes, for example `http request -> handler -> service -> repository`, `changed symbol -> direct callers -> affected tests`, or `review path for this PR`. This is metadata over graph, not new edge kind and not runtime tracing requirement in v1.
- `flow membership`: join row in `flow_memberships` that says one node participates in one flow, with optional `position`, `role`, and metadata. `membership` here never means user/team/account membership.
- `community`: unordered cluster of related nodes/modules/files found by some graph algorithm or heuristic, for example SCC/cycle cluster, package cluster, or architecture slice. Community says "these belong together"; flow says "these form ordered path".
- `embeddings`: optional vector search data for retrieval/ranking only. Not required for core build/update/query path.

### Core Design Rule

- Avoid feature growth without signal quality gains
- Prioritize better ranking
- Prioritize better context
- Prioritize better signals

---

## Roadmap Layout

- Part IV. Remaining context continuity roadmap: ICM-inspired memory follow-on roadmap (ICM-B through ICM-H)
- Part V. Remaining focused follow-up patches: Retrieval Follow-Up Patch remainder, Runtime Event Enrichment and Graph Linking Patch, Rust Reachability Guard Patch, Shared Parser Query Migration Patch, Context Escalation Contract Patch, Dynamic Agent Policy and Hook Enforcement Patch, Graph Store Corruption Recovery Patch, SQLite Connection Concurrency Policy Patch remainder
- Part XI. Tokenizer-backed context budget accounting

## Cross-Cutting Track Map

- Retrieval and search follow-ups: Retrieval Follow-Up Patch remainder
- Context continuity and runtime memory: ICM-inspired memory follow-on roadmap (ICM-B through ICM-H), Runtime Event Enrichment and Graph Linking Patch
- Graph safety and workflow: Context Escalation Contract Patch, Graph Store Corruption Recovery Patch, SQLite Connection Concurrency Policy Patch remainder
- Rust parser correctness: Rust Reachability Guard Patch, Shared Parser Query Migration Patch
- Agent policy and enforcement: Dynamic Agent Policy and Hook Enforcement Patch

---

## Part IV — Context Continuity and Memory

Use this part for session persistence, saved artifacts, retrieval-backed resume, and long-lived memory work.

### Context-Mode and Continuity Roadmap

These phases cover continuity storage, session lifecycle, retrieval-backed restoration, memory quality, and longer-term cross-session intelligence.

Extend Atlas with context-mode persistence and session continuity without mixing those concerns into graph database.

This backlog covers pieces needed for:

- artifact persistence
- session continuity
- resume snapshots
- retrieval-backed restoration

Core Design Rules:

- DO NOT store saved context in graph database
- DO NOT replay raw command history into future sessions
- ALWAYS restore context through retrieval
- ALWAYS store large outputs outside model context
- KEEP graph storage, content storage, and session storage as separate systems
- KEEP continuity best-effort; never block primary CLI/MCP flow on session persistence failure
- KEEP retrieval lexical and local first; embeddings are optional later, not required for v1 context-mode completion

---

Phases CM14 (Decision Memory) and CM15 (Agent-Aware Context), ICM-0 (universal MCP session capture fallback), and ICM-A (shared memory surface) are shipped. See SHIPPED.md for details. Remaining memory quality work (ICM-B through ICM-H) is tracked in the ICM roadmap below.

### ICM-Inspired Memory Follow-On Roadmap

Use this section to merge compatible parts of `atlas-icm-inspired-memory-roadmap.md` into the existing continuity architecture.

Priority order below is implementation order. Extend shipped Phase CM14 and Phase CM15 behavior. Do not introduce a parallel memory stack that conflicts with current `session.db` / `context.db` / `worldtree.db` boundaries.

This grouped roadmap covers the full source document at theme level, except shell-first simplifications requested here: no slash-command track, no skill-install track, and no web dashboard track.

Before implementing any ICM checklist item:

- read the parent ICM section `Rules:` block
- treat `Rules:` bullets as mandatory constraints, not tasks
- do not mark `Rules:` bullets done; they are never checklist items
- implement checklist items only under `Implementation structure` and `completion criteria`
- if a checklist item conflicts with `Rules:`, follow `Rules:` and update the checklist wording before implementation

#### ICM-B — Memory Curation, Decay, Health, and Consolidation

Rules:

- add memory decay config with safe defaults and explicit critical-memory protection
- preserve deterministic maintenance structure from source roadmap so cleanup work stays implementation-ready after source file deletion
- do not auto-prune `critical` memories by default
- do not hard-delete linked saved-context artifacts unless explicitly requested
- do not make health scoring or consolidation depend on opaque LLM behavior
- do not mutate state during `--dry-run`

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-B1 — Decay policy config and scoring

- [ ] add memory decay config to `.atlas/config.toml`
- [ ] add default retention policy with `critical` never auto-pruned, `high` long retention, `normal` normal retention, and `low` short retention
- [ ] add config fields `memory.decay.enabled`, `memory.decay.low_days`, `memory.decay.normal_days`, `memory.decay.high_days`, and `memory.decay.critical_never_prune`
- [ ] validate retention days as positive integers and fail `atlas doctor` clearly on invalid config
- [ ] add `atlas memory decay` with `--dry-run`, `--topic`, and `--json`
- [ ] calculate updated `decay_score` without deleting rows

##### ICM-B2 — Stale, prune, and health commands

- [ ] add `atlas memory stale` with `--topic`, `--scope`, and `--json`
- [ ] list only stale memories and never report critical memories as auto-prune candidates
- [ ] add `atlas memory prune` with `--dry-run`, `--topic`, `--importance`, `--older-than`, and `--json`
- [ ] delete only memories marked pruneable by policy and require explicit override before any critical-memory prune path exists
- [ ] add memory health categories `healthy`, `stale`, `noisy`, `duplicated`, `orphaned`, and `oversized`
- [ ] detect low-importance old memories, repeated memories, missing `source_id` references, noisy topics, and topics with no critical decisions
- [ ] add `atlas memory health` with `--topic`, `--scope`, and `--json`
- [ ] emit actionable suggestions and exact follow-up commands in human output

##### ICM-B3 — Deterministic consolidation

- [ ] add deterministic consolidation planner grouping by topic, similar title, similar body, same source id, and same feedback or decision category
- [ ] preserve all `source_id` references in consolidation plan output
- [ ] add `atlas memory consolidate` with `--topic`, `--scope`, `--dry-run`, and `--json`
- [ ] in dry-run mode, report kept ids, merged ids, and source preservation without mutating storage
- [ ] add apply mode that creates consolidated memory, marks merged rows as superseded, and stores supersession links `old_memory_id`, `new_memory_id`, and `reason`
- [ ] make recall prefer consolidated rows while allowing explicit inspection of superseded rows later

##### ICM-B completion criteria

- [ ] default decay config loads without a memory section present
- [ ] `atlas memory decay --dry-run` reports protected critical memories and updated scores
- [ ] `atlas memory prune --importance low --dry-run` reports only pruneable low-priority rows
- [ ] `atlas memory health --topic hooks` returns deterministic findings and suggestions
- [ ] consolidation preserves source references and leaves dry-run fully read-only

#### ICM-C — Feedback Memory and Analysis Confidence Adjustment

Rules:

- add feedback storage and search for predicted vs actual outcomes, correction text, related symbol/file, and `source_id`
- keep feedback as first-class deterministic correction memory rather than loose comments or opaque notes
- do not let feedback override deterministic graph evidence silently
- do not lower confidence without explicit matching evidence
- do not couple feedback storage to graph tables or graph-node lifecycle

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-C1 — Feedback storage and search model

- [ ] create `feedback_records` table with `id`, `repo_root`, `session_id`, `tool_name`, `analysis_kind`, `predicted`, `actual`, `correction`, `related_symbol`, `related_file`, `source_id`, `created_at`, and `metadata_json`
- [ ] add FTS index for `predicted`, `actual`, `correction`, `related_symbol`, and `related_file`
- [ ] keep feedback searchable by symbol, file, correction text, and analysis kind

##### ICM-C2 — CLI and MCP feedback commands

- [ ] add `atlas feedback record` with required `--predicted` and `--actual`
- [ ] add optional `--correction`, `--tool`, `--analysis-kind`, `--symbol`, `--file`, `--source-id`, and `--json`
- [ ] add `atlas feedback search <query>` with filters `--tool`, `--analysis-kind`, `--symbol`, `--file`, `--limit`, and `--json`
- [ ] add `atlas feedback stats` with deterministic summary and `--json`
- [ ] add MCP `feedback_record` using same service layer and validation contract

##### ICM-C3 — Confidence adjustment integration

- [ ] query feedback before returning results from `atlas analyze dead-code`, `atlas analyze remove`, `atlas analyze safety`, and `atlas refactor remove-dead --dry-run`
- [ ] lower confidence only when prior feedback indicates false positives for same symbol, file, pattern, or analysis kind
- [ ] expose `feedback_evidence` in analysis JSON whenever scoring changes
- [ ] add config flag `analysis.feedback_adjustment.enabled`

##### ICM-C completion criteria

- [ ] missing `--predicted` or `--actual` fails validation
- [ ] feedback search returns predicted, actual, correction, related symbol/file, score, and created time
- [ ] empty feedback DB returns stable zero-count stats
- [ ] stored false-positive feedback can lower confidence only when evidence actually matches

#### ICM-D — Wake-Up Packs and Session Start Recall

Rules:

- define a bounded wake-up pack that summarizes current focus, critical memories, recent decisions, recent feedback, graph readiness, changed files, and retrieval hints
- keep wake-up path compact, retrieval-backed, and consistent with resume architecture already shipped in continuity work
- do not inline raw large artifacts into wake-up or resume payloads
- do not block session start on wake-up generation failure
- do not replay raw command history as wake-up context

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-D1 — Wake-up pack model

- [ ] define `WakePack` model with `repo_root`, `session_id`, `frontend`, `current_focus`, `recent_decisions`, `critical_memories`, `recent_feedback`, `active_memoir_concepts`, `changed_files`, `graph_readiness`, `retrieval_hints`, and `generated_at`
- [ ] bound wake-up pack size through config and central budget policy
- [ ] serialize wake-up packs to stable JSON

##### ICM-D2 — CLI and MCP wake-up

- [ ] add `atlas wake-up` with flags `--topic`, `--session`, `--frontend`, `--max-items`, and `--json`
- [ ] pull wake-up content from memory, feedback, session resume, and graph readiness services
- [x] add MCP `wake_up` with compact default output, retrieval hints, and source ids instead of raw artifact bodies (shipped early by ICM-0E)

##### ICM-D3 — Hook integration

- [ ] call wake-up generation from `SessionStart` hook paths where host supports it
- [ ] attach wake-up packs to session resume only through bounded injection paths
- [ ] store wake-up generation success or failure metadata in session events
- [ ] keep hook failures non-blocking and best-effort

##### ICM-D completion criteria

- [ ] `atlas wake-up --topic hooks` prioritizes topic-relevant memories and feedback
- [ ] wake-up output references large artifacts by `source_id` only
- [ ] hook failures do not stop host command flow
- [ ] snapshot tests cover empty, normal, and large sessions

#### ICM-E — Cross-Session Recall Quality and Optional Semantic Recall

Rules:

- improve recall ranking with topic match, importance, recency, scope visibility, and source-backed evidence
- preserve lexical-first default and make cross-session recall quality measurable before adding vector complexity
- do not make embeddings required for baseline memory recall
- do not let vector scores outrank exact lexical or stronger structural evidence by default
- do not widen frontend-private or session-private recall unless caller explicitly asks for it

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-E1 — Cross-session recall quality

- [ ] extend memory recall across prior repo sessions while preserving agent/frontend visibility boundaries
- [ ] rank recall by topic match, importance, recency, scope visibility, and source-backed evidence
- [ ] make system capable of recalling past sessions without mixing raw session history into future context
- [ ] optimize context selection so recall surfaces the highest-signal memories first

##### ICM-E2 — Optional semantic and vector recall

- [ ] add config `memory.embedding.enabled`, `memory.embedding.provider`, `memory.embedding.model`, `memory.embedding.dimension`, `memory.search.hybrid_weight_fts`, and `memory.search.hybrid_weight_vector`
- [ ] keep embeddings disabled by default and require explicit opt-in
- [ ] add `memory_embeddings` table with `memory_id`, `embedding_model`, `dimension`, `vector_blob`, and `created_at`
- [ ] reject vector inserts when configured dimension does not match stored dimension
- [ ] add `atlas memory recall <query> --hybrid` using reciprocal-rank fusion only after lexical evaluation and budget metrics exist
- [ ] keep graph-backed and exact lexical evidence stronger than vector-only matches by default

##### ICM-E completion criteria

- [ ] baseline memory recall works lexically with no embedding provider configured
- [ ] enabling embeddings without provider or valid dimension fails clearly
- [ ] hybrid recall returns ranking explanation fields without burying exact keyword hits
- [ ] cross-session recall respects `global`, `project`, `session`, and `frontend` visibility boundaries

#### ICM-F — Memoir Concept Graph as Separate Knowledge Layer

Rules:

- add separate memoir tables, concepts, relations, and graph ids outside the code graph schema
- keep memoir path explicit and bounded so semantic memory does not leak into code graph semantics
- do not merge memoir concepts into code graph `nodes` and `edges`
- do not allow unbounded custom relation types by default
- do not auto-create missing concepts unless caller explicitly opts in

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-F1 — Memoir schema and vocabulary

- [ ] create `memoir_graphs`, `memoir_concepts`, and `memoir_relations` tables separate from code graph storage
- [ ] store relation fields `graph_id`, `source_concept_id`, `target_concept_id`, `relation_type`, `confidence`, `source_id`, `created_at`, and `metadata_json`
- [ ] add controlled relation vocabulary `depends_on`, `part_of`, `contradicts`, `refines`, `replaces`, `caused_by`, `fixed_by`, `blocked_by`, `decided_by`, and `related_to`
- [ ] normalize aliases such as `replaced_by` and `separate_from` with explicit direction or tagging rules
- [ ] reject unknown relation types unless config later enables custom relations

##### ICM-F2 — CLI and MCP memoir commands

- [ ] add `atlas memoir create <name>` with `--description`, `--scope`, and `--json`
- [ ] add `atlas memoir add-concept <graph> <name> <description>` with `--kind`, `--source-id`, and `--json`
- [ ] add `atlas memoir link <graph> <source> <target> --relation <type>` with `--confidence`, `--source-id`, and `--json`
- [ ] add `atlas memoir inspect <concept>` with `--graph`, `--depth`, `--relation`, and `--json`
- [ ] add MCP `memoir_create`, `memoir_add_concept`, `memoir_link`, and `memoir_inspect` as thin wrappers over same service layer

##### ICM-F completion criteria

- [ ] duplicate memoir graph names fail deterministically in same repo and scope
- [ ] `atlas memoir link A B --relation depends_on` succeeds and invalid relation names fail clearly
- [ ] bounded inspect output includes relation direction and source evidence ids
- [ ] code graph queries remain unaware of memoir tables unless explicit memoir surface is invoked

#### ICM-G — Code Overview Memory for External Analysis

Rules:

- add a graph-linked overview memory layer for project, package, module, file, symbol, function, and method descriptions
- let Atlas export bounded analysis packets and ingest externally produced LLM or human analysis
- keep Atlas non-LLM by default; no built-in model provider, prompt runner, or hidden network call in this track
- use overview memory as readable package/module/spec documentation and as guidance inside context/review payloads
- keep overview records tied to commit SHA, graph freshness, canonical repo paths, and qualified names so stale guidance is detectable
- do not store overview bodies in `worldtree.db`
- do not make overview text a graph fact or merge it into graph `nodes` and `edges`
- do not let stale overview records appear as fresh context without explicit stale metadata
- do not require embeddings or an LLM provider for baseline overview search
- do not add compatibility shims for old overview schemas until a first stable schema ships
- do not auto-run external analyzers during normal `build`, `update`, `query`, `context`, or MCP flows unless user explicitly configures hooks

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-G1 — Overview domain model and subject identity

- [ ] define `OverviewSubjectKind` enum with exact values `project`, `package`, `module`, `file`, `symbol`, `function`, and `method`
- [ ] define `OverviewFreshness` enum with exact values `fresh`, `possibly_stale`, `stale`, `orphaned`, and `unverified`
- [ ] define `OverviewSourceKind` enum with exact values `external_llm`, `manual`, and `imported`
- [ ] define `OverviewSubject` with `kind`, `repo_root`, `commit_sha`, optional `package_name`, optional `module_path`, optional `canonical_file_path`, optional `qualified_name`, optional `node_kind`, optional `line_start`, optional `line_end`, and optional `content_hash`
- [ ] validate `project` subjects require only `repo_root` and `commit_sha`
- [ ] validate `package` subjects require `package_name`
- [ ] validate `module` subjects require `module_path`
- [ ] validate `file` subjects require `canonical_file_path`
- [ ] validate `symbol`, `function`, and `method` subjects require `qualified_name`, `canonical_file_path`, `line_start`, and `line_end`
- [ ] canonicalize every subject path through `atlas_repo::CanonicalRepoPath` before hashing, persistence, dedupe, lookup, or stale matching
- [ ] define deterministic `subject_id` as versioned hash over `repo_root`, `kind`, canonical subject fields, and schema version
- [ ] reject local path-normalization helpers and add tests proving `./src/lib.rs` and `src/lib.rs` resolve to same subject identity
- [ ] define `OverviewRecord` with subject, title, summary, description, responsibilities, flow, inputs, outputs, invariants, gotchas, examples, tags, source metadata, freshness, confidence, source ids, and timestamps
- [ ] represent list fields as typed vectors at service boundary and JSON arrays at storage boundary
- [ ] bound title, summary, description, and each list field through central budget policy before storage
- [ ] route oversized overview bodies through existing content-store artifact routing and store only preview plus `source_id` in overview table
- [ ] add unit tests for subject validation, enum parsing, subject-id stability, path canonicalization, and budget truncation metadata

##### ICM-G2 — Overview storage schema and migrations

- [ ] add overview tables to continuity-owned storage, preferably the memory/session-side persistence used by ICM-A unless a dedicated continuity DB is justified in code comments
- [ ] create `overview_records` table with `id`, `repo_root`, `subject_id`, `subject_kind`, `package_name`, `module_path`, `canonical_file_path`, `qualified_name`, `node_kind`, `line_start`, `line_end`, `content_hash`, `title`, `summary`, `description_preview`, `responsibilities_json`, `flow_json`, `inputs_json`, `outputs_json`, `invariants_json`, `gotchas_json`, `examples_json`, `tags_json`, `source_kind`, `analyzer_name`, `analyzer_version`, `model_name`, `commit_sha`, `graph_last_indexed_at`, `freshness`, `confidence`, `source_id`, `supersedes_id`, `created_at`, `updated_at`, and `metadata_json`
- [ ] create `overview_record_fts` over `title`, `summary`, `description_preview`, `responsibilities_json`, `flow_json`, `invariants_json`, `gotchas_json`, and `tags_json`
- [ ] add indexes for `repo_root`, `subject_id`, `subject_kind`, `canonical_file_path`, `qualified_name`, `commit_sha`, `freshness`, `source_kind`, and `updated_at`
- [ ] add uniqueness rule for active records by `repo_root`, `subject_id`, `commit_sha`, and `source_kind` unless `supersedes_id` is set
- [ ] add supersession support so re-ingest creates a new active record and points to previous active record through `supersedes_id`
- [ ] preserve old overview records for audit unless user later adds explicit prune command
- [ ] add storage API methods `insert_overview_record`, `get_overview_record`, `search_overview_records`, `list_stale_overview_records`, `supersede_overview_record`, and `mark_overview_freshness`
- [ ] make storage API reject invalid enum strings and malformed JSON arrays before writing
- [ ] update `atlas db check` to validate overview schema, FTS integrity, orphan `source_id` references, invalid enum values, and noncanonical path rows
- [ ] add migration golden tests, in-memory storage tests, FTS search tests, and db-check failure fixture tests

##### ICM-G3 — Analysis packet export contract

- [ ] add `OverviewExportRequest` with `scope`, `subjects`, `changed_only`, `since`, `limit`, `include_code_spans`, `include_callers`, `include_callees`, `include_tests`, `max_tokens`, and `json` fields
- [ ] add `OverviewAnalysisPacket` with `schema_version`, `repo_root`, `commit_sha`, graph provenance, freshness metadata, subject, concise graph evidence, bounded code excerpt, callers, callees, related files, test adjacency, and retrieval hints
- [ ] support export scopes `project`, `packages`, `modules`, `files`, `symbols`, `functions`, `methods`, and `changed`
- [ ] implement `atlas overview export --scope <scope> --json`
- [ ] implement `atlas overview export --subject <qualified_name_or_path> --json`
- [ ] implement `atlas overview export --changed --since <rev> --json`
- [ ] use graph-backed context resolution first for symbols and functions, then companion content lookup only for docs/config/assets surfaced by graph/context evidence
- [ ] include canonical subject identity in every packet so ingest can validate exact target later
- [ ] include stable packet id as hash over subject id, commit SHA, selected evidence ids, and export schema version
- [ ] include `safe_to_answer`, graph freshness, omitted counts, and budget-hit metadata in every packet
- [ ] fail export clearly when graph readiness is `corrupt`; allow stale export only with explicit stale metadata
- [ ] add tests for project export, package export, changed-only export, function export, ambiguous subject failure, stale graph metadata, and budget truncation

##### ICM-G4 — External analysis ingest contract

- [ ] define `OverviewAnalysisInput` JSON schema with `schema_version`, `packet_id`, subject identity, title, summary, description, responsibilities, flow, inputs, outputs, invariants, gotchas, examples, tags, confidence, analyzer metadata, and optional source artifact ids
- [ ] implement `atlas overview ingest <path>` for JSON file input
- [ ] implement `atlas overview ingest -` for stdin input
- [ ] validate input `schema_version` exactly before field validation
- [ ] validate `packet_id` when present and return clear mismatch error when packet id does not match exported packet metadata
- [ ] validate subject exists in current or indexed graph by canonical file path and qualified name where applicable
- [ ] validate line ranges still overlap same graph node before marking ingested record `fresh`
- [ ] mark record `possibly_stale` when commit SHA differs but subject identity still resolves
- [ ] mark record `orphaned` when canonical path or qualified name no longer resolves
- [ ] mark record `unverified` for manual/imported records without packet id
- [ ] enforce confidence range `0.0..=1.0`
- [ ] require `summary` for all records and require either `description` or at least one non-empty detail array
- [ ] reject unknown top-level fields unless `metadata_json.extra` explicitly captures them through a controlled importer path
- [ ] run overview text through central redaction policy before persistence
- [ ] route large descriptions and examples through content-store when over inline budget
- [ ] add JSON schema fixture tests for valid external LLM result, valid manual import, missing summary, bad confidence, subject mismatch, stale commit, orphan subject, and secret redaction

##### ICM-G5 — Freshness, commit update, and refresh planning

- [ ] add `OverviewFreshnessService` that compares overview records against current graph readiness, current commit SHA, changed files, content hashes, and line-span overlap
- [ ] implement direct stale marking when record canonical file path changed since stored commit
- [ ] implement direct stale marking when record content hash differs from current file hash
- [ ] implement direct stale marking when record qualified name no longer resolves
- [ ] implement `possibly_stale` marking for callers, containing modules, containing packages, and project records when dependent files changed
- [ ] implement orphan marking for deleted files, removed symbols, and renamed subjects without confident canonical target
- [ ] use `detect_changes`/history data when available; fall back to git diff only through existing repo/change services, not ad hoc shell parsing in service code
- [ ] implement `atlas overview stale` with filters `--subject-kind`, `--file`, `--package`, `--module`, `--since`, `--limit`, and `--json`
- [ ] implement `atlas overview refresh-plan --changed --since <rev> --json`
- [ ] make refresh plan output include subject id, stale reason, suggested export command, affected dependents, previous record id, and priority
- [ ] rank refresh plan priority by direct change before dependent change, subject kind specificity, current context relevance, confidence, and updated_at age
- [ ] keep refresh planning read-only unless caller passes explicit apply flag for freshness marking
- [ ] add tests for changed function, changed file, deleted file, renamed file, caller possibly stale, package possibly stale, and no-op unchanged commit

##### ICM-G6 — Overview CLI read and maintenance surfaces

- [ ] implement `atlas overview show <subject>` resolving exact subject id, qualified name, canonical file path, package name, or module path
- [ ] make ambiguous `show` results fail with candidate list and required disambiguation fields
- [ ] implement `atlas overview search <query>` with filters `--subject-kind`, `--freshness`, `--package`, `--module`, `--file`, `--source-kind`, `--limit`, and `--json`
- [ ] rank search by exact subject match, exact title/tag match, FTS score, freshness, confidence, recency, and subject specificity
- [ ] implement `atlas overview list` with filters `--subject-kind`, `--freshness`, `--source-kind`, `--older-than`, `--newer-than`, and `--json`
- [ ] implement `atlas overview delete <overview_id> --dry-run --json`
- [ ] require exact overview id for delete and keep routed content-store artifacts unless explicit artifact-delete behavior is added later
- [ ] implement `atlas overview export-docs --format markdown --output <path>` for readable project/package/module specs
- [ ] make exported docs group by project, package, module, file, then symbol, with stale records labeled visibly
- [ ] make human output use `println!`/`eprintln!`; reserve tracing macros for diagnostics
- [ ] add CLI smoke tests and JSON snapshot tests for show, search, list, stale, refresh-plan, delete dry-run, and markdown export

##### ICM-G7 — Context engine and retrieval integration

- [ ] extend context request controls with `include_overviews`, `overview_limit`, `overview_freshness`, `overview_subject_kinds`, and `overview_max_bytes`
- [ ] include fresh overview records in `atlas context`, `atlas review-context`, MCP `get_context`, and MCP `get_review_context` when they match selected symbols, files, modules, packages, or changed files
- [ ] include stale records only when request allows stale overviews and always emit stale reason
- [ ] rank overview context by exact symbol match, exact file match, containing module/package, changed-file relevance, freshness, confidence, and recency
- [ ] emit overview selection reasons such as `same_symbol`, `same_file`, `containing_module`, `package_summary`, `project_summary`, and `changed_dependency`
- [ ] keep overview payload preview-only by default and expose `source_id` for full body retrieval
- [ ] merge overview records under existing graph/content/session budget policy instead of adding separate truncation rules
- [ ] ensure overview text cannot override graph facts in risk, impact, dead-code, or refactor analysis
- [ ] add tests for graph-only context, overview-only companion context, mixed graph/overview context, stale overview exclusion, stale overview explicit inclusion, and budget trimming

##### ICM-G8 — MCP parity and external analyzer handoff

- [ ] add MCP `overview_export` with same request fields, defaults, and JSON shape as CLI export
- [ ] add MCP `overview_ingest` with same validation and error shape as CLI ingest
- [ ] add MCP `overview_search` with same filters and ranking evidence as CLI search
- [ ] add MCP `overview_show` with same ambiguity behavior as CLI show
- [ ] add MCP `overview_refresh_plan` with same read-only default behavior as CLI refresh-plan
- [ ] keep MCP default output compact and include `source_id`, freshness, selection reason, and next export command where relevant
- [ ] add parity tests proving CLI and MCP record shapes, validation failures, freshness states, and default limits match
- [ ] expose optional external analyzer handoff only as packet generation plus documented command contract, not as built-in model execution

##### ICM-G9 — Hook and manual trigger integration

- [ ] add config section `overview` with `enabled`, `auto_export_on_commit`, `auto_mark_stale_on_commit`, `external_analyzer_command`, `ingest_after_external_command`, `max_subjects_per_run`, and `max_packet_bytes`
- [ ] default `overview.enabled = true`, `auto_export_on_commit = false`, `auto_mark_stale_on_commit = true`, and `ingest_after_external_command = false`
- [ ] validate external analyzer command is absent or explicit string path/command; never infer model provider from environment variables
- [ ] add hook integration that can run `overview refresh-plan` after commit or manual hook trigger
- [ ] keep hook failures best-effort and non-blocking for git/host flow
- [ ] store hook-generated packet exports in content-store when oversized and reference them by `source_id`
- [ ] record hook outcome as session event with command, status, packet count, ingested count, stale count, and source ids
- [ ] add manual command `atlas overview run-external --dry-run --json` that prints exact external command invocations without executing them
- [ ] add apply mode for `run-external` that executes configured command, requires JSON output, validates ingest, and reports per-subject success/failure
- [ ] add tests for default config, invalid config, dry-run command generation, nonblocking hook failure, successful external ingest, malformed external output, and max-subject cap

##### ICM-G10 — Overview docs, fixtures, and release gate

- [ ] add `wiki/overview-memory.md` documenting storage ownership, external analyzer contract, JSON schemas, freshness states, context integration, hook behavior, and CLI/MCP parity
- [ ] add reusable fixtures for project overview, package overview, module overview, file overview, function overview, stale function overview, orphaned symbol overview, manual overview, and oversized overview body
- [ ] add JSON snapshots for `overview export`, `overview ingest --json`, `overview show --json`, `overview search --json`, `overview stale --json`, `overview refresh-plan --json`, and MCP overview tools
- [ ] add markdown snapshot for `overview export-docs --format markdown`
- [ ] add schema evolution note that first stable schema is `schema_version = 1` and later breaking changes must ship migration or explicit import rejection
- [ ] update `wiki/memory-architecture.md` to explain overview memory as evidence-linked code documentation separate from generic memories, feedback records, and memoir concepts
- [ ] define release gate `ICM Overview Memory Complete`
- [ ] require for release gate: storage schema, export/ingest contracts, freshness planner, CLI read surfaces, context integration, MCP parity, hook/manual trigger path, docs, fixtures, and JSON snapshots
- [ ] require for release gate: no overview body writes to `worldtree.db`, no built-in LLM provider, no stale overview emitted as fresh, and no path-derived identity without `CanonicalRepoPath`

##### ICM-G completion criteria

- [ ] `atlas overview export --scope functions --json` emits bounded packets with subject ids, commit SHA, graph freshness, and budget metadata
- [ ] `atlas overview ingest analysis.json` stores a fresh record when packet id, commit SHA, canonical path, qualified name, and line span match current graph
- [ ] `atlas overview ingest analysis.json` marks record `possibly_stale` when commit SHA differs but subject still resolves
- [ ] `atlas overview ingest analysis.json` marks record `orphaned` when subject no longer resolves
- [ ] `atlas overview refresh-plan --changed --since HEAD~1 --json` reports directly stale and possibly stale subjects with suggested export commands
- [ ] `atlas context --include-overviews` includes fresh overview records with selection reasons and source ids
- [ ] MCP overview tools match CLI JSON defaults and validation behavior
- [ ] `atlas db check` reports invalid overview schema, invalid enum values, orphan source ids, and noncanonical overview paths
- [ ] `./scripts/test-workspace-summary.sh` passes after overview memory implementation

#### ICM-H — Shell-First Install Modes, TUI, Docs, and Release Gates

Rules:

- add install/init mode split for `mcp`, `hook`, `cli`, and `all`, with idempotent generation and dry-run preview
- keep shell-first and TUI-first operational structure from source roadmap while dropping slash-command, skill, and dashboard work
- do not add slash-command generators or skill-install surfaces for this track
- do not add web dashboard routes for memory inspection in this track
- do not build TUI surfaces before core service contracts and tests stabilize
- do not introduce host-specific command generators that bypass shared service logic

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-H1 — Shell-first install and init modes

- [ ] add supported `atlas init --mode` values `mcp`, `hook`, `cli`, and `all`
- [ ] make each mode idempotent and emit files to be created during `--dry-run`
- [ ] ensure `--mode all` installs only MCP config, hooks, and CLI config relevant to shell-first memory workflows

##### ICM-H2 — TUI only, read-only first

- [ ] add `atlas memory tui` with read-only browsing for memories, topics, feedback, memoir concepts, overview records, health findings, and saved artifacts
- [ ] add filters for topic, scope, importance, and frontend
- [ ] add overview filters for subject kind, freshness, package, module, file, source kind, and updated time
- [ ] keep first version non-mutating and smoke-testable without panic

##### ICM-H3 — Tests, docs, and release gates

- [ ] create reusable fixtures for critical decision memory, low-priority stale memory, dead-code false-positive feedback, memoir dependency graph, overview memory records, wake-up pack with saved artifact references, and frontend-private memory
- [ ] snapshot JSON output for `atlas memory store --json`, `atlas memory recall --json`, `atlas memory health --json`, `atlas feedback record --json`, `atlas feedback search --json`, `atlas memoir inspect --json`, `atlas overview show --json`, `atlas overview search --json`, and `atlas wake-up --json`
- [ ] add `wiki/memory-architecture.md` documenting memory DB ownership, importance and decay policy, scope and visibility rules, feedback integration, memoir graph separation, overview memory separation, wake-up behavior, and CLI/MCP mapping
- [ ] define release gate `ICM Memory Layer Complete`
- [ ] require for release gate: CLI and MCP memory store/recall parity, importance and decay policies, feedback-adjusted analysis, memoir typed relations, overview memory export/ingest/context parity, wake-up packs without raw large content, health audit coverage, shared/private visibility rules, complete docs, and JSON snapshot coverage

##### ICM-H completion criteria

- [ ] every new shell-first memory command has CLI smoke coverage
- [ ] every MCP memory tool has handler tests and parity assertions where applicable
- [ ] `cargo test --workspace` passes with fixtures and JSON snapshots committed
- [ ] no memory feature writes directly to graph DB
- [ ] no large artifact is inlined into wake-up or resume output by default
- [ ] no overview memory feature treats external analysis text as authoritative graph fact

---

## Part V — Follow-Up Patches

Use these patch sections for focused improvements that cut across existing roadmap phases without rewriting phase scope.

### Retrieval Follow-Up Patch

These are the high-value retrieval/indexing improvements still missing or only partially specified after the current v3 plan.

They are meant to strengthen Atlas’s retrieval/content sidecar without changing the graph-first core.

#### Patch R7 — Later experimental post-retrieval compaction

This is not core and should stay late, but it is a useful optional experiment once retrieval and context engine behavior are stable.

- [ ] add backlog item for post-retrieval compaction experiment
- [ ] only evaluate after:
  - [ ] hybrid retrieval is stable
  - [ ] context engine output quality is stable
  - [ ] token-efficiency metrics exist
- [ ] keep initial experiment strictly optional
- [ ] require evidence that compaction reduces tokens without harming answer quality
- [ ] do not let this replace retrieval filtering or graph-based selection

Why:
- useful possible optimization later
- should not destabilize current graph-first + retrieval-assisted architecture

#### Patch completion criteria

This patch is complete when:

- [x] retrieval/content index has explicit searchable state
- [x] retrieval indexing has batch and chunk guardrails
- [x] embedding dimension rules are explicit and enforced
- [x] retrieval backend capabilities are validated, not assumed
- [x] stable `chunk_id` exists and is used for dedupe/reuse
- [x] retrieval/token-efficiency benchmarks are in place
- [ ] optional post-retrieval compaction is tracked as a late experiment only

---


### Runtime Event Enrichment and Graph Linking Patch

Atlas already has session events, adapter extraction helpers, content-store artifact routing, resume snapshots, saved-context retrieval, and context-engine saved-context merge. Do not replace that foundation with a parallel extractor system. Extend it with deterministic enrichment that turns runtime activity into bounded, graph-aware memory while preserving the existing storage boundaries: graph facts stay in `worldtree.db`, large/runtime artifacts stay in `context.db`, and session timelines stay in `session.db`.

#### Patch X1 — Scope and crate boundary

- [ ] define this as enrichment over existing `atlas-session`, `atlas-contentstore`, and `atlas-adapters`
- [ ] avoid creating `packages/atlas-extractor` unless extraction logic grows large enough to justify a separate crate
- [ ] if a new crate is created later, require it to depend on service APIs, not write SQLite directly
- [ ] keep extractor pipeline deterministic, local, and non-LLM
- [ ] keep extractor best-effort; extraction failure must not block primary CLI/MCP tool output
- [ ] keep raw runtime output out of graph DB
- [ ] document storage ownership:
  - [ ] `worldtree.db` stores static code graph facts only
  - [ ] `session.db` stores bounded event metadata and references
  - [ ] `context.db` stores large artifacts, chunks, previews, and searchable runtime text

Why:
- existing continuity architecture already solved session/content boundaries
- a parallel extractor crate or DB path would duplicate behavior and increase drift

#### Patch X2 — Raw input envelope and deterministic event enrichment

- [ ] define a `RuntimeInput` / `RawActivityInput` envelope for enrichment:
  - [ ] `frontend` (`cli`, `mcp`, adapter host)
  - [ ] `session_id`
  - [ ] `repo_root`
  - [ ] `input_kind`
  - [ ] `tool_or_command`
  - [ ] `status`
  - [ ] `stdout_preview`
  - [ ] `stderr_preview`
  - [ ] `artifact_source_id`
  - [ ] `files`
  - [ ] `metadata`
  - [ ] `created_at`
- [ ] define enriched output that maps onto existing `NewSessionEvent` payloads:
  - [ ] `event_type`
  - [ ] `summary`
  - [ ] `symbols`
  - [ ] `file_paths`
  - [ ] `source_ids`
  - [ ] `classification`
  - [ ] `confidence`
  - [ ] `metadata`
- [ ] enrich existing event constructors rather than bypassing them:
  - [ ] `extract_cli_event`
  - [ ] `extract_graph_event`
  - [ ] `extract_context_event`
  - [ ] `extract_reasoning_event`
  - [ ] `extract_user_event`
  - [ ] `extract_tool_event`
  - [ ] `normalize_event`
- [ ] keep outputs canonical JSON so existing event hashing and dedupe remain stable
- [ ] add tests proving same input produces same enriched event and hash

Why:
- enrichment should preserve existing event persistence and dedupe semantics
- deterministic input/output keeps resume snapshots stable

#### Patch X3 — Rule-based classification

- [ ] add bounded rule-based classifiers for runtime activity:
  - [ ] panic
  - [ ] exception
  - [ ] stacktrace
  - [ ] compiler error
  - [ ] test failure
  - [ ] test success
  - [ ] build success
  - [ ] deprecation warning
  - [ ] unused/dead-code warning
  - [ ] permission denied
  - [ ] command timeout
  - [ ] graph stale/readiness warning
  - [ ] retrieval/content-store failure
- [ ] map classifications to existing `SessionEventType` values where possible:
  - [ ] `ERROR`
  - [ ] `COMMAND_RUN`
  - [ ] `COMMAND_FAIL`
  - [ ] `CONTEXT_REQUEST`
  - [ ] `REASONING_RESULT`
  - [ ] `FILE_READ`
  - [ ] `FILE_WRITE`
  - [ ] `GRAPH_BUILD`
  - [ ] `GRAPH_UPDATE`
- [ ] add new event types only when existing types cannot represent the event safely
- [ ] include classification metadata instead of exploding event-type count:
  - [ ] `classification.kind`
  - [ ] `classification.severity`
  - [ ] `classification.rule_id`
  - [ ] `classification.matched_fields`
- [ ] add tests for error parsing, warning parsing, test summary parsing, and no-match behavior

Why:
- event type should stay stable; detailed meaning belongs in structured metadata
- deterministic classifiers provide useful memory without LLM inference

#### Patch X4 — Artifact routing before session insertion

- [ ] run all raw stdout/stderr/tool-result blobs through existing content-store routing before session insertion
- [ ] define routing thresholds through the central budget policy:
  - [ ] `small_output_bytes`
  - [ ] `preview_output_bytes`
  - [ ] `large_output_bytes`
  - [ ] `max_runtime_artifact_bytes`
- [ ] keep session event payloads bounded:
  - [ ] small output may be stored inline only when safe and redacted
  - [ ] medium output stores preview plus `source_id`
  - [ ] large output stores pointer only
- [ ] use `ContentStore::route_output` / saved-context artifact routing instead of a new artifact path
- [ ] index routed artifacts with metadata:
  - [ ] `session_id`
  - [ ] `source_type`

#### Patch X5 — Graph linking without storing runtime data in graph DB

- [ ] link enriched events to graph facts by stable identifiers, not raw node IDs alone
- [ ] store links in session/content side tables, not `worldtree.db`
- [ ] define link records:
  - [ ] `event_id`
  - [ ] `session_id`
  - [ ] `repo_root`
  - [ ] `qualified_name`
  - [ ] `canonical_file_path`
  - [ ] optional `node_id`
  - [ ] optional `file_id`
  - [ ] `link_kind`
  - [ ] `confidence`
  - [ ] `graph_last_indexed_at`
- [ ] prefer canonical identifiers:
  - [ ] canonical repo path
  - [ ] qualified name
  - [ ] kind
  - [ ] line span when available
- [ ] treat `node_id` and `file_id` as cache hints only because graph rebuilds can change row IDs
- [ ] make graph linking best-effort:
  - [ ] events with no graph target remain valid runtime memory
  - [ ] ambiguous symbols store candidate list and ambiguity metadata
  - [ ] stale graph state records `safe_to_answer=false` for link-derived claims when needed
- [ ] add tests for exact symbol, file path, ambiguous symbol, stale graph, and graph-missing cases

Why:
- runtime memory should be graph-aware without mutating graph facts
- stable identifiers survive rebuilds better than SQLite row IDs

#### Patch X6 — Readiness, identity, and budget integration

- [ ] run graph linking only through canonical graph readiness state
- [ ] define behavior by execution state:
  - [ ] `fresh` -> resolve and link normally
  - [ ] `stale` -> link with freshness warning and stale metadata
  - [ ] `partial` -> link only when completeness requirements are met
  - [ ] `corrupt` -> skip graph linking and store runtime event without graph links
- [ ] require canonical path identity before any event/file/artifact key hashing
- [ ] apply central budget policy to:
  - [ ] classifier input bytes
  - [ ] number of symbols extracted
  - [ ] number of file paths extracted
  - [ ] number of graph lookup candidates
  - [ ] number of links stored
  - [ ] artifact preview bytes
- [ ] emit enrichment budget metadata:
  - [ ] `budget_hit`
  - [ ] `partial`
  - [ ] `safe_to_answer`
  - [ ] omitted symbol/file/link counts
- [ ] add tests for stale/partial/corrupt graph behavior and budget truncation

Why:
- runtime enrichment must follow the same safety rules as graph-backed tools
- extraction can otherwise become another unbounded path

#### Patch X7 — Context-engine integration

- [ ] extend context engine to include enriched runtime events only when requested or relevant
- [ ] add request controls:
  - [ ] `include_runtime_events`
  - [ ] `runtime_event_limit`
  - [ ] `runtime_artifact_limit`
  - [ ] `runtime_since`
  - [ ] `runtime_session_id`
- [ ] retrieve runtime memory by:
  - [ ] linked symbol
  - [ ] canonical file path
  - [ ] session id
  - [ ] classification kind
  - [ ] artifact source id
- [ ] merge runtime memory under graph/content companion policy
- [ ] expose source kind:
  - [ ] `runtime_event`
  - [ ] `runtime_artifact`
  - [ ] `saved_context`
- [ ] include selection reason and ranking evidence:
  - [ ] same symbol
  - [ ] same file
  - [ ] recent error
  - [ ] same session
  - [ ] direct artifact reference
- [ ] keep runtime context bounded and preview-only by default
- [ ] add tests for context with graph-only, saved-context-only, runtime-event-only, and mixed graph/runtime inputs

Why:
- runtime memory is useful only when it participates in context selection
- it must not bypass existing context budgets or ranking rules

#### Patch X8 — CLI, MCP, and hook integration

- [ ] integrate enrichment with existing CLI adapter event flow
- [ ] integrate enrichment with MCP tool handler boundaries
- [ ] keep MCP session event persistence best-effort and non-blocking
- [ ] avoid duplicating `save_context_artifact`; reuse existing tool and content routing
- [ ] update hook integration roadmap so host hooks emit enriched inputs through service APIs
- [ ] ensure generated hooks never write SQLite directly
- [ ] add command/tool metadata for:
  - [ ] command start
  - [ ] command finish
  - [ ] tool result
  - [ ] permission decision
  - [ ] compaction boundary
  - [ ] session end
  - [ ] error/failure
- [ ] add integration tests for CLI, MCP, and bridge-file fallback event enrichment

Why:
- runtime memory should come from existing adapters and hooks
- host-specific capture gaps must reduce enrichment quality, not break commands

#### Patch X9 — Resume snapshot enrichment

- [ ] include enriched runtime signals in resume snapshots:
  - [ ] recent errors
  - [ ] recent failed commands
  - [ ] recent successful build/test summaries
  - [ ] linked symbols
  - [ ] linked files
  - [ ] artifact references
  - [ ] active unresolved runtime issues
- [ ] group by category and severity
- [ ] include retrieval hints instead of raw artifact content
- [ ] cap snapshot contribution by budget policy
- [ ] make snapshot rendering deterministic
- [ ] add snapshot tests for enriched errors, artifact references, and linked symbols

Why:
- resume should recover useful runtime state without replaying history
- enriched events make snapshots more useful while staying compact

#### Patch X completion criteria

- [ ] runtime enrichment extends existing session/content/adapters architecture without replacing it
- [ ] no runtime data is stored in graph DB
- [ ] large runtime outputs route through content store before session insertion
- [x] enriched events are deterministic, bounded, redacted, and deduplicated
- [ ] redaction policy supports runtime-loaded external rule files with validation
- [ ] event-to-graph links use stable identifiers and treat row IDs as optional cache hints
- [ ] graph linking obeys readiness state and budget policy
- [ ] context engine can merge runtime events/artifacts with graph and saved context under one bounded ranking policy
- [ ] CLI, MCP, and hook flows feed enrichment best-effort
- [ ] resume snapshots include compact enriched runtime signals
- [ ] tests cover classification, artifact routing, graph linking, context integration, and resume enrichment

---


### Rust Reachability Guard Patch

Atlas Rust call resolution can over-report cross-file references for orphan files because `same_package` heuristics use package ownership plus simple-name matching, but do not verify crate-root module reachability. A file can be outside the compiled module tree and still accumulate inbound graph edges. `cross_file_links` then treats those heuristic edges as evidence that the file is connected.

The current `resolve_same_package_target` in `atlas-engine/src/call_resolution.rs` filters candidates by `owner_id` match (Cargo package) and then optionally by same directory. Neither check requires the candidate file to be reachable from any crate root via `mod` declarations. This lets stale, deleted, or orphan files remain as resolution targets as long as they share a Cargo package.

Design overview:

Two complementary data structures carry the fix:

1. **`CrateReachabilityIndex`** — built once per Cargo package during the parse/build phase. Stores the set of canonical file paths reachable from each crate root (lib, main, example, test, bench) within the package. Built by walking `mod` declarations in parsed ASTs rather than filesystem scanning. Lives in `atlas-engine` or `atlas-parser`; never written to `worldtree.db`.

2. **`ReachabilityGuard`** — thin wrapper passed into `resolve_same_package_target` alongside the existing `Store` and owner cache. Given a `(caller_file, candidate_file)` pair, it answers `is_reachable(candidate_file, from_crate_root_of: caller_file)`. Returns `false` when the index is absent (safe default: block heuristic edge rather than assume live).

Edge provenance gets one new field: `reachability_checked: bool`. When `true` and `same_package` tier is set, the candidate passed crate-root reachability. When `false`, the edge is a legacy heuristic edge emitted before the guard existed.

#### Patch R1 — `CrateReachabilityIndex` model and builder

- [ ] define `CrateReachabilityIndex` struct in `atlas-engine` (or `atlas-parser` if mod-walk lives there):
  - [ ] `owner_id: String` — Cargo manifest key, matches existing `owner_id` field
  - [ ] `crate_roots: Vec<CrateRoot>` — one entry per compiled crate target
  - [ ] each `CrateRoot`:
    - [ ] `root_file: CanonicalRepoPath` — e.g. `src/lib.rs`, `src/main.rs`, `examples/foo.rs`
    - [ ] `crate_kind: CrateKind` — `Lib`, `Bin`, `Example`, `Test`, `Bench`
    - [ ] `reachable_files: HashSet<CanonicalRepoPath>` — all files reachable via `mod` from this root
- [ ] implement `CrateReachabilityIndex::build(owner_id, manifest_path, parsed_files)`:
  - [ ] identify crate roots by standard Cargo layout heuristics: `src/lib.rs`, `src/main.rs`, `examples/*.rs`, `tests/*.rs`, `benches/*.rs`
  - [ ] respect `[[bin]]`, `[[example]]`, `[[test]]`, `[[bench]]` `path` overrides from `Cargo.toml` when parsed
  - [ ] walk `mod <name>;` declarations in each root file using already-parsed AST nodes (no re-parse)
  - [ ] resolve sibling `mod` paths relative to declaring file using Rust module path rules (`mod foo;` → `foo.rs` or `foo/mod.rs`)
  - [ ] recursively follow `mod` declarations up to a configurable depth cap (default: 64 levels)
  - [ ] treat `mod foo { ... }` inline modules as transparent (they do not add a new file, all their declarations remain in the declaring file)
  - [ ] treat unresolvable `mod` targets as absent rather than erroring out; record them in `unresolved_mods` for diagnostics
  - [ ] all file paths stored as `CanonicalRepoPath` via `atlas_repo::CanonicalRepoPath`
- [ ] expose `is_file_reachable(file: &CanonicalRepoPath) -> bool` helper that checks across all `CrateRoot` entries in the index
- [ ] expose `reachable_from_same_root(caller: &CanonicalRepoPath, candidate: &CanonicalRepoPath) -> bool` — returns `true` only when both files appear in the same `CrateRoot.reachable_files` set
- [ ] add unit tests:
  - [ ] standard `src/lib.rs` layout with one level of `mod`
  - [ ] nested `mod foo { mod bar; }` inline with sibling file
  - [ ] multi-target package: lib + bin + example each have separate reachable sets
  - [ ] orphan `.rs` file in same package directory not reachable from any crate root
  - [ ] unresolvable `mod` target is recorded but does not panic or block other mods
  - [ ] path identity: same file via different path strings produces one entry

Why:
- `owner_id` covers Cargo package membership, not Rust module-tree membership
- index must be built from AST, not filesystem, to stay consistent with parsed graph facts

#### Patch R2 — `ReachabilityGuard` and integration into `resolve_same_package_target`

- [ ] define `ReachabilityGuard` in `atlas-engine`:
  - [ ] wraps `HashMap<String, CrateReachabilityIndex>` keyed by `owner_id`
  - [ ] `is_reachable_from_same_root(caller: &str, candidate: &str) -> ReachabilityResult`
  - [ ] `ReachabilityResult` variants: `Reachable`, `Unreachable`, `IndexAbsent`
  - [ ] treat `IndexAbsent` as non-reachable (safe default: do not emit heuristic edge without evidence)
- [ ] build `ReachabilityGuard` once per engine build/update run, before resolution pass
- [ ] thread `ReachabilityGuard` into `resolve_same_package_target` alongside existing `owner_cache`
- [ ] update `resolve_same_package_target` resolution order:
  1. filter candidates by `owner_id` (existing step — coarse package filter)
  2. apply receiver-hint filtering (existing step — keep)
  3. **new**: filter `same_owner_matches` to retain only candidates where `ReachabilityGuard::is_reachable_from_same_root(caller, candidate)` returns `Reachable`
  4. apply existing same-dir tie-break on the reachability-filtered set
  5. if reachability index is absent (`IndexAbsent`), fall back to existing behavior but mark edge with `reachability_checked: false`
- [ ] add `reachability_checked: bool` to edge metadata or edge extra fields (stored in existing `metadata` JSON or new column)
- [ ] add regression tests:
  - [ ] orphan file in same Cargo package is rejected as same-package target after reachability filtering
  - [ ] live file reachable via `mod` chain is accepted as same-package target
  - [ ] receiver-hint still narrows candidates correctly after reachability filtering
  - [ ] absent index falls back gracefully and does not panic

Why:
- package membership alone is too broad; reachability narrows to files the compiler actually sees
- `IndexAbsent` fallback prevents breaking existing resolution for languages or layouts where index is not built

#### Patch R3 — Edge provenance and `cross_file_links` filtering

- [ ] audit `cross_file_links` query for Rust heuristic-edge false positives:
  - [ ] identify whether `cross_file_links` joins only on edge existence or also on confidence tier
  - [ ] determine whether filtering at read time or write time is safer given incremental update semantics
- [ ] decide and document filter strategy:
  - [ ] **preferred**: filter at write time — do not persist `same_package` edges for unreachable candidates; `cross_file_links` naturally sees correct graph
  - [ ] **acceptable fallback**: filter at read time — add `reachability_checked = true` predicate to `cross_file_links` query for Rust `same_package` edges
  - [ ] document chosen strategy in a code comment near the `cross_file_links` query
- [ ] ensure incremental update removes stale node rows and their inbound `same_package` edges when a file is deleted
  - [ ] verify existing node deletion cascade covers edge rows; add explicit edge cleanup if missing
- [ ] expose edge provenance in `cross_file_links` output:
  - [ ] add `confidence_tier` to `CrossFileLink` result struct if not already present
  - [ ] add `reachability_checked` flag to `CrossFileLink` when available
- [ ] add tests:
  - [ ] orphan Rust file shows zero `cross_file_links` inbound edges after reachability-gated build
  - [ ] deleted Rust file shows zero `cross_file_links` results after incremental refresh removes its nodes
  - [ ] import-backed edge (`use` / `extern crate`) still appears in `cross_file_links` regardless of reachability guard

Why:
- `cross_file_links` is the user-visible surface; false-positive heuristic edges here mislead dead-code and impact analysis
- write-time filtering is cleaner than read-time masking

#### Patch R4 — Diagnostics and observability

- [ ] expose reachability index stats in `atlas doctor` / `atlas db_check` output:
  - [ ] number of Cargo packages with reachability index built
  - [ ] number of packages where index build failed or was skipped
  - [ ] number of unresolved `mod` targets across all packages
  - [ ] number of `same_package` edges emitted with `reachability_checked: true` vs `false`
- [ ] expose reachability status per file in `atlas status --json` or a dedicated debug command:
  - [ ] file is reachable from which crate root(s)
  - [ ] file has no reachable crate root (orphan)
- [ ] log reachability index build failures at `warn` level with package path; do not fail the build
- [ ] add MCP `doctor` response fields for reachability index health when data is available

Why:
- operators need to see whether the guard is active and which packages lack an index
- silent guard absence produces the same false positives as before, so visibility is required

#### Patch R completion criteria

- [ ] `CrateReachabilityIndex` model exists and is built from parsed AST `mod` declarations
- [ ] `ReachabilityGuard` wraps the index and answers caller/candidate reachability queries
- [ ] `resolve_same_package_target` in `atlas-engine/src/call_resolution.rs` filters candidates through `ReachabilityGuard` before emitting `same_package` edges
- [ ] `same_package` edges carry `reachability_checked` provenance
- [ ] `cross_file_links` does not claim orphan Rust files are connected after a reachability-gated build
- [ ] incremental refresh removes deleted-file nodes and clears their inbound edges
- [ ] `atlas doctor` reports reachability index coverage and unresolved mod counts
- [ ] tests cover: orphan file rejection, live file acceptance, receiver-hint interaction, absent index fallback, deleted-file cleanup, and `cross_file_links` false-positive regression

---

Rust Parser Query-Backed Extraction Patch is shipped. See SHIPPED.md for details.

### Shared Parser Query Migration Patch

Implement this only after Patch Q is complete. Rust is the pilot for shared query infrastructure and capture conventions. This patch migrates the remaining tree-sitter-backed language parsers to the same query-backed extraction model without changing parser public APIs, database schemas, or graph output contracts.

The migration rule is: `.scm` queries identify language syntax facts; Rust code in each language parser still owns Atlas graph semantics, including qualified names, parent scopes, edge kinds, confidence tiers, source metadata, and language-specific heuristics. Do not replace semantic resolution with query captures alone.

Check https://github.com/helix-editor/helix/tree/master/runtime/queries for scm grammar references for the languages.

Use Helix queries only as grammar reference for tree-sitter node names and scope patterns, especially `runtime/queries/*/tags.scm` and `runtime/queries/*/locals.scm`. Do not copy Helix query files verbatim unless license handling is added, because Helix is MPL-2.0. Atlas query files must be authored for Atlas captures.

#### Patch SQ1 — Shared query contract and migration harness

- [x] document the shared query-backed parser contract in `packages/atlas-parser/README.md`:
  - [x] query files live under `packages/atlas-parser/queries/<language>.scm`
  - [x] capture names use the `@atlas.*` namespace
  - [x] queries capture syntax facts only
  - [x] language parser code maps captures into `Node`, `Edge`, and `ParsedFile`
  - [x] language parser public APIs remain unchanged
- [x] harden shared query helpers created by Patch Q:
  - [x] support loading one static query per language via `include_str!`
  - [x] expose helper for capture lookup by exact capture name
  - [x] expose helper for optional and required captures with clear test failures
  - [x] expose helper to sort captures by byte range for deterministic output
  - [x] expose helper to preserve source-order traversal when multiple query matches overlap
- [x] define common capture naming conventions:
  - [x] `@atlas.definition.function`
  - [x] `@atlas.definition.method`
  - [x] `@atlas.definition.class`
  - [x] `@atlas.definition.module`
  - [x] `@atlas.definition.struct`
  - [x] `@atlas.definition.enum`
  - [x] `@atlas.definition.interface`
  - [x] `@atlas.definition.trait`
  - [x] `@atlas.definition.constant`
  - [x] `@atlas.definition.variable`
  - [x] `@atlas.import`
  - [x] `@atlas.call`
  - [x] `@atlas.reference`
  - [x] `@atlas.name`
  - [x] `@atlas.parameters`
  - [x] `@atlas.return_type`
  - [x] `@atlas.receiver`
- [x] add query helper tests:
  - [x] invalid query text returns a clear error
  - [x] missing required capture returns a clear error
  - [x] optional capture absence does not fail
  - [x] capture order is deterministic across repeated runs
  - [x] overlapping captures preserve match order before graph builder filtering
- [x] add migration checklist comments in each remaining parser file naming the existing manual extraction responsibilities before refactor starts

Why:
- prevents each language migration from inventing incompatible capture names
- makes query-backed parser behavior testable before broad parser churn
- keeps graph semantics explicit and separate from tree-sitter syntax matching

#### Patch SQ2 — Migrate C-family compiled language parsers

- [x] migrate `packages/atlas-parser/src/lang/c.rs`:
  - [x] add `packages/atlas-parser/queries/c.scm`
  - [x] query functions, structs, enums, typedefs, includes, and calls
  - [x] preserve existing C qualified names and `NodeKind` choices
  - [x] preserve existing include/import edge behavior
  - [x] preserve existing same-file call behavior
  - [x] keep `tests/fixtures/c/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] migrate `packages/atlas-parser/src/lang/cpp.rs`:
  - [x] add `packages/atlas-parser/queries/cpp.scm`
  - [x] query functions, methods, classes, structs, namespaces, includes, and calls
  - [x] preserve existing C++ qualified names and `NodeKind` choices
  - [x] preserve existing namespace and class parent scope behavior
  - [x] keep `tests/fixtures/cpp/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] migrate `packages/atlas-parser/src/lang/csharp.rs`:
  - [x] add `packages/atlas-parser/queries/csharp.scm`
  - [x] query namespaces, classes, interfaces, methods, fields, using directives, and calls
  - [x] preserve existing C# qualified names and `NodeKind` choices
  - [x] preserve existing test detection behavior
  - [x] keep `tests/fixtures/csharp/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] run tests after this batch:
  - [x] `cargo test -p atlas-parser lang::c`
  - [x] `cargo test -p atlas-parser lang::cpp`
  - [x] `cargo test -p atlas-parser lang::csharp`
  - [x] `cargo test -p atlas-parser --test parser_golden`

Why:
- C-family parsers share enough syntax shape to validate query conventions for compiled languages
- batch keeps blast radius bounded before dynamic-language migrations

#### Patch SQ3 — Migrate JVM and static OO language parsers

- [x] migrate `packages/atlas-parser/src/lang/java.rs`:
  - [x] add `packages/atlas-parser/queries/java.scm`
  - [x] query packages, imports, classes, interfaces, enums, methods, fields, and calls
  - [x] preserve existing Java qualified names and `NodeKind` choices
  - [x] preserve existing parent scope behavior
  - [x] keep `tests/fixtures/java/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] migrate `packages/atlas-parser/src/lang/scala.rs`:
  - [x] add `packages/atlas-parser/queries/scala.scm`
  - [x] query packages, imports, classes, objects, traits, functions, vals, vars, and calls
  - [x] preserve existing Scala qualified names and `NodeKind` choices
  - [x] preserve existing object/class/trait scope behavior
  - [x] keep `tests/fixtures/scala/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] run tests after this batch:
  - [x] `cargo test -p atlas-parser lang::java`
  - [x] `cargo test -p atlas-parser lang::scala`
  - [x] `cargo test -p atlas-parser --test parser_golden`

Why:
- Java and Scala exercise package/import scope semantics after C-family migration
- keeps static OO migration separate from JavaScript/TypeScript complexity

#### Patch SQ4 — Migrate JavaScript and TypeScript parsers

- [x] migrate shared JavaScript/TypeScript parser code in `packages/atlas-parser/src/lang/javascript.rs`:
  - [x] add `packages/atlas-parser/queries/javascript.scm`
  - [x] add `packages/atlas-parser/queries/typescript.scm`
  - [x] query imports, exports, functions, arrow functions assigned to names, classes, methods, variables, and calls
  - [x] preserve existing JavaScript qualified names and `NodeKind` choices
  - [x] preserve existing TypeScript qualified names and `NodeKind` choices
  - [x] preserve existing JSX/TSX support behavior
  - [x] preserve existing call/reference confidence tiers
  - [x] keep `tests/fixtures/javascript/*.golden.json` unchanged unless a semantic fix is explicitly itemized
  - [x] keep `tests/fixtures/typescript/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] run tests after this batch:
  - [x] `cargo test -p atlas-parser lang::javascript`
  - [x] `cargo test -p atlas-parser --test parser_golden`

Why:
- JavaScript and TypeScript share parser code and must migrate together to avoid divergent behavior
- this batch validates query helpers against two grammars behind one language module

#### Patch SQ5 — Migrate dynamic language parsers

- [x] migrate `packages/atlas-parser/src/lang/python.rs`:
  - [x] add `packages/atlas-parser/queries/python.scm`
  - [x] query imports, classes, functions, methods, assignments, and calls
  - [x] preserve existing Python qualified names and `NodeKind` choices
  - [x] preserve existing indentation/scope behavior from AST parentage
  - [x] keep `tests/fixtures/python/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] migrate `packages/atlas-parser/src/lang/ruby.rs`:
  - [x] add `packages/atlas-parser/queries/ruby.scm`
  - [x] query requires, modules, classes, instance methods, singleton methods, constants, and calls
  - [x] preserve existing Ruby qualified names and `NodeKind` choices
  - [x] preserve existing current-owner behavior
  - [x] keep `tests/fixtures/ruby/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] migrate `packages/atlas-parser/src/lang/php.rs`:
  - [x] add `packages/atlas-parser/queries/php.scm`
  - [x] query namespaces, uses, classes, interfaces, traits, functions, methods, constants, and calls
  - [x] preserve existing PHP qualified names and `NodeKind` choices
  - [x] preserve existing PHP language mode setup
  - [x] keep `tests/fixtures/php/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] migrate `packages/atlas-parser/src/lang/bash.rs`:
  - [x] add `packages/atlas-parser/queries/bash.scm`
  - [x] query function definitions, command invocations, variables, and source/import-like commands
  - [x] preserve existing Bash qualified names and `NodeKind` choices
  - [x] keep `tests/fixtures/bash/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] run tests after this batch:
  - [x] `cargo test -p atlas-parser lang::python`
  - [x] `cargo test -p atlas-parser lang::ruby`
  - [x] `cargo test -p atlas-parser lang::php`
  - [x] `cargo test -p atlas-parser lang::bash`
  - [x] `cargo test -p atlas-parser --test parser_golden`

Why:
- dynamic languages rely heavily on scope heuristics, so they should migrate after query helpers are proven
- batch validates method/function owner handling across multiple dynamic grammar styles

#### Patch SQ6 — Migrate data, markup, and style parsers where queries add value

- [x] evaluate query migration for `packages/atlas-parser/src/lang/json.rs`:
  - [ ] migrate to `packages/atlas-parser/queries/json.scm` only if it reduces manual traversal without losing object/key path semantics
  - [x] otherwise document why JSON remains manual
  - [x] keep `tests/fixtures/json/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] evaluate query migration for `packages/atlas-parser/src/lang/toml.rs`:
  - [ ] migrate to `packages/atlas-parser/queries/toml.scm` only if it reduces manual traversal without losing table/key path semantics
  - [x] otherwise document why TOML remains manual
  - [x] keep `tests/fixtures/toml/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] evaluate query migration for `packages/atlas-parser/src/lang/html.rs`:
  - [x] migrate to `packages/atlas-parser/queries/html.scm` only if query captures improve element/script/style extraction
  - [ ] otherwise document why HTML remains manual
  - [x] keep `tests/fixtures/html/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] evaluate query migration for `packages/atlas-parser/src/lang/css.rs`:
  - [x] migrate to `packages/atlas-parser/queries/css.scm` only if query captures improve selector/rule extraction
  - [ ] otherwise document why CSS remains manual
  - [x] keep `tests/fixtures/css/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] evaluate query migration for `packages/atlas-parser/src/lang/markdown.rs`:
  - [ ] migrate to `packages/atlas-parser/queries/markdown.scm` only if tree-sitter-md query behavior stays stable for malformed shorter inputs
  - [x] otherwise document why Markdown remains manual
  - [x] preserve current decision to avoid unstable incremental reuse for Markdown unless separately fixed
  - [x] keep `tests/fixtures/markdown/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [x] run tests after this batch:
  - [x] `cargo test -p atlas-parser lang::json`
  - [x] `cargo test -p atlas-parser lang::toml`
  - [x] `cargo test -p atlas-parser lang::html`
  - [x] `cargo test -p atlas-parser lang::css`
  - [x] `cargo test -p atlas-parser lang::markdown`
  - [x] `cargo test -p atlas-parser --test parser_golden`

Why:
- data/markup/style parsers may not benefit equally from queries
- this batch requires explicit migrate-or-document decisions instead of forced churn

#### Patch SQ completion criteria

- [ ] every non-Rust parser has either an Atlas-owned query file or a documented reason to remain manual
- [x] all migrated parsers use shared query helpers instead of ad hoc `tree_sitter::QueryCursor` code
- [x] all migrated parsers keep public parser APIs unchanged
- [x] golden outputs remain unchanged unless semantic fixes are explicitly itemized in the corresponding patch
- [x] parser docs describe the query-backed extraction contract and capture naming convention
- [x] `cargo test -p atlas-parser --test parser_golden` passes after each migration batch
- [ ] `cargo test -p atlas-parser` passes after the final migration batch
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes after the final migration batch
- [ ] `cargo fmt --all` has been run after the final migration batch

---


### Context Escalation Contract Patch

Atlas has compact context tools, review context, symbol lookup, neighbor tools, and wider traversal tools, but the preferred order is currently only hinted in prompts and installed instructions. Make the core agent workflow explicit: start with the smallest bounded graph context that can answer the question, then escalate only when evidence says broader context is needed.

#### Patch E1 — Define minimal-context-first workflow

- [ ] document canonical escalation order for review/change tasks:
  - [ ] `detect_changes` when files are unknown
  - [ ] `get_minimal_context` for first bounded triage
  - [ ] `get_review_context` only when changed-symbol, neighbor, or risk detail is needed
  - [ ] `explain_change` when deterministic risk/test-gap explanation is needed
  - [ ] `get_impact_radius` when explicit blast radius is needed
- [ ] document canonical escalation order for symbol/usage tasks:
  - [ ] `query_graph` / `resolve_symbol` first
  - [ ] `symbol_neighbors` for direct callers/callees/tests
  - [ ] `get_context` for bounded ranked context
  - [ ] `traverse_graph` only when one-hop context is insufficient
- [ ] define allowed reasons to escalate:
  - [ ] ambiguous symbol resolution
  - [ ] truncated result
  - [ ] missing caller/callee/test evidence
  - [ ] cross-file or cross-package risk
  - [ ] explicit user request for broader context
  - [ ] safety-critical uncertainty
- [ ] define anti-patterns:
  - [ ] starting review with full review context when minimal context is enough
  - [ ] using traversal before symbol resolution
  - [ ] using file search before graph tools answer structural questions
  - [ ] broad traversal without a bounded max depth and max nodes

Why:
- reduces token load and noisy context
- keeps graph workflows deterministic and cheap by default

#### Patch E2 — Surface contract in MCP, prompts, and installed instructions

- [ ] update MCP tool descriptions to mention minimal-first escalation where relevant
- [ ] update `review_change` prompt to make minimal context first a requirement, not just a recommendation
- [ ] update `inspect_symbol` prompt to require direct-neighbor context before wider traversal
- [x] update installed AGENTS instructions to state escalation order clearly
- [ ] update README MCP workflow section to match same order
- [ ] ensure wording is consistent across CLI install block, MCP prompts, and README

Why:
- agents follow tool descriptions and prompts more reliably than implicit design intent
- one workflow description prevents drift across docs and MCP metadata

#### Patch E2.5 — Enforce minimal-context-first inside higher-level tools

- [ ] require higher-level tools to start from minimal bounded context internally unless explicitly bypassed:
  - [ ] `get_review_context`
  - [ ] `explain_change`
  - [ ] `get_impact_radius`
  - [ ] `analyze_safety`
  - [ ] `analyze_remove`
  - [ ] `analyze_dead_code`
  - [ ] `analyze_dependency`
  - [ ] refactor planning tools
- [ ] define explicit bypass reasons:
  - [ ] user requested full context
  - [ ] minimal context is truncated
  - [ ] minimal context reports ambiguity
  - [ ] tool requires full impact graph by contract
  - [ ] configured safety policy requires broader context
- [ ] include metadata showing whether minimal context was used, bypassed, or escalated:
  - [ ] `minimal_context_used`
  - [ ] `minimal_context_bypassed`
  - [ ] `escalation_reason`
  - [ ] `next_tools`
- [ ] add tests proving review/analyze/impact tools do not over-fetch when minimal context is sufficient

Why:
- workflow guidance is weaker than internal enforcement
- higher-level tools should not silently bypass bounded triage

#### Patch E3 — Add escalation metadata and tests where practical

- [ ] include response metadata that helps decide whether to escalate:
  - [ ] `truncated`
  - [ ] `omitted_count`
  - [ ] `ambiguity`
  - [ ] `next_tools`
  - [ ] `recommended_escalation_reason`
- [ ] ensure `get_minimal_context` reports when review context would add useful detail
- [ ] ensure `symbol_neighbors` reports when traversal may be needed because caps were hit
- [ ] add prompt/registry snapshot tests for minimal-first contract wording
- [ ] add MCP response tests for escalation metadata on truncated/ambiguous outputs

Why:
- tools should tell agents when more context is justified
- escalation should be evidence-driven, not habit-driven

#### Patch E completion criteria

- [ ] minimal-context-first contract is documented as required workflow
- [ ] higher-level tools internally start from minimal context or emit explicit bypass metadata
- [ ] MCP prompts, tool descriptions, README, and installed AGENTS instructions agree
- [ ] graph/context responses expose enough metadata to justify escalation
- [ ] tests protect contract wording and escalation metadata

---

### Dynamic Agent Policy and Hook Enforcement Patch

Atlas already installs static AGENTS/CLAUDE instructions and platform hook files, but current workflow policy still lives mostly in static text. Add one runtime policy surface plus hard hook enforcement so agents can load fresh Atlas workflow guidance at session start without trying to make markdown executable.

#### Patch A1 — Canonical runtime policy contract

- [ ] define compact `AgentInstructionsPolicy` model in shared service code with fields:
  - [ ] `policy_version`
  - [ ] `generated_at`
  - [ ] `frontend`
  - [ ] `policy_mode`
  - [ ] `required_first_step`
  - [ ] `required_tool_order`
  - [ ] `protected_tools`
  - [ ] `forbidden_patterns`
  - [ ] `fallback_behavior`
  - [ ] `trust_notes`
  - [ ] `source`
- [ ] keep policy payload deterministic and compact enough for hook/session injection
- [ ] make one shared Rust service produce policy for both MCP tool calls and `atlas hook`
- [ ] version policy explicitly so hooks can detect stale cached payloads
- [ ] add serde round-trip tests for policy schema stability

Why:
- runtime workflow policy should have one source of truth
- MCP tool output, hook preload, prompts, and installed instructions must not drift

#### Patch A2 — MCP `agent_instructions` tool surface

- [ ] add `agent_instructions` to MCP tool registry in `packages/atlas-mcp/src/tools/registry.rs`
- [ ] add dispatch arm in `packages/atlas-mcp/src/tools/dispatch.rs`
- [ ] implement handler that returns current `AgentInstructionsPolicy`
- [ ] accept explicit inputs:
  - [ ] `frontend`
  - [ ] `policy_mode`
  - [ ] `include_fallback_static_rules`
  - [ ] `output_format`
- [ ] default output to compact agent-facing payload suitable for session preload
- [ ] include TOON and JSON parity tests for tool output
- [ ] add registry snapshot test so installed instructions and MCP registry stay aligned

Why:
- agent needs runtime policy as normal Atlas surface, not hidden ad hoc hook text
- hook runner should reuse same policy source returned by MCP

#### Patch A3 — Installed instruction bootstrap text

- [ ] update install-generated instruction block in `packages/atlas-cli/src/install/instructions.rs`
- [ ] replace duplicated workflow detail with explicit bootstrap rule:
  - [ ] call `agent_instructions` before substantive repo exploration
  - [ ] use static AGENTS rules only when runtime policy is unavailable
  - [ ] keep graph-first and minimal-context-first invariants in static text
- [ ] keep injected section idempotent under existing instruction markers
- [ ] add install test proving stale injected section is replaced with new bootstrap wording
- [ ] add install test proving user-authored content before and after injected section is preserved

Why:
- static markdown should bootstrap runtime policy, not duplicate mutable operational rules
- install flow already owns AGENTS/CLAUDE injected guidance and should remain source for static bootstrap text

#### Patch A4 — Platform hook preload integration

- [ ] extend `packages/atlas-cli/src/install/platform_hooks.rs` generated Copilot hook config to preload policy on:
  - [ ] `SessionStart`
  - [ ] `UserPromptSubmit`
- [ ] extend generated Claude hook config to preload policy on:
  - [ ] `SessionStart`
  - [ ] `UserPromptSubmit`
  - [ ] `InstructionsLoaded`
- [ ] extend generated Codex hook config to preload policy on:
  - [ ] `SessionStart`
  - [ ] `UserPromptSubmit`
- [ ] extend shared `.atlas/hooks/atlas-hook` runner so preload path calls shared Rust policy service instead of duplicating JSON assembly in shell
- [ ] cache last successful compact policy payload under `.atlas/hooks/lib/` with version/hash metadata
- [ ] define bounded cache TTL or invalidation rule so long sessions can refresh policy safely
- [ ] add tests for generated hook configs and runner output after install

Why:
- existing install-generated hook path already exists and should carry runtime policy preload
- session-start and prompt-submit are strongest points for loading fresh policy before work begins

#### Patch A5 — Hard enforcement at hook boundary

- [ ] make hook enforcement check whether current session has loaded valid policy version before protected tool execution
- [ ] define initial protected tool set:
  - [ ] `query_graph`
  - [ ] `get_context`
  - [ ] `get_review_context`
  - [ ] `get_minimal_context`
  - [ ] `get_impact_radius`
  - [ ] `explain_change`
  - [ ] graph-backed analysis tools
  - [ ] refactor planning tools
- [ ] define exempt diagnostic/repair tools that remain fail-open when policy preload fails:
  - [ ] `status`
  - [ ] `doctor`
  - [ ] `db_check`
  - [ ] `debug_graph`
  - [ ] `build_or_update_graph`
- [ ] return explicit enforcement decision metadata:
  - [ ] `policy_loaded`
  - [ ] `policy_version`
  - [ ] `enforcement_mode`
  - [ ] `blocked_reason`
  - [ ] `fallback_active`
- [ ] record enforcement events through existing adapter/session APIs; do not let hooks write SQLite directly
- [ ] add integration test proving protected tool is blocked before preload and allowed after preload

Why:
- AGENTS text alone cannot guarantee runtime behavior
- hook boundary is correct deterministic enforcement point for required policy preload

#### Patch A6 — Fallback and degraded-mode behavior

- [ ] define explicit fallback path when runtime policy fetch fails:
  - [ ] static AGENTS/install rules remain active
  - [ ] protected tools use configured fail-open or fail-closed behavior by class
  - [ ] fallback state is surfaced in metadata instead of silent skip
- [ ] ensure fallback does not bypass graph-readiness checks or existing safety gates
- [ ] ensure fallback path remains deterministic when cache exists but live fetch fails
- [ ] add tests for:
  - [ ] live fetch failure with valid cache
  - [ ] live fetch failure without cache
  - [ ] stale cache version rejection
  - [ ] explicit degraded metadata in hook/session output

Why:
- runtime policy fetch can fail and behavior must stay explicit, bounded, and safe
- degraded mode should not silently weaken existing Atlas safety contracts

#### Patch A7 — Prompt and documentation consistency

- [ ] update MCP prompts in `packages/atlas-mcp/src/prompts.rs` to mention `agent_instructions` as first runtime step where relevant
- [ ] update installed AGENTS instructions to reference runtime-policy bootstrap and fallback rules
- [ ] update README and wiki MCP workflow docs to match same wording
- [ ] ensure graph/content companion wording and minimal-context-first wording stay consistent with runtime-policy contract
- [ ] add snapshot tests protecting prompt/install/doc wording from drift

Why:
- prompts and installed instructions are agent-facing control surfaces and must agree
- runtime policy is only useful if every workflow surface points to same first-step contract

#### Patch A completion criteria

- [ ] `agent_instructions` exists as MCP tool with stable compact output
- [ ] installed AGENTS/bootstrap text tells agents to call `agent_instructions` first and defines fallback clearly
- [ ] install-generated Copilot/Claude/Codex hooks preload runtime policy on session/prompt start
- [ ] protected Atlas tools are blocked when required policy preload has not happened
- [ ] fallback mode is explicit, deterministic, and covered by tests
- [ ] prompts, installed instructions, README, and wiki workflow docs agree on runtime-policy-first behavior
- [ ] adapter/session event flow records policy preload and enforcement decisions without direct hook SQLite writes

---

### Graph Store Corruption Recovery Patch

Atlas already has partial readiness plumbing in `packages/atlas-core/src/readiness.rs` (`IntegrityState`, `GraphExecutionState`) and shared error-code text in `packages/atlas-core/src/health.rs` / `docs/error_codes.md`. It can report SQLite failures, stale graph state, interrupted builds, orphan nodes, and dangling edges, but graph-store corruption policy for `.atlas/worldtree.db` is still incomplete. Detection must lead to one clear outcome: quarantine unusable graph data, rebuild from repository source, and block graph-backed answers while stored graph facts are unsafe.

Current gap to close after Phase C1: physical SQLite corruption and logical graph inconsistency are now split into stable `sqlite_corrupt` vs `logical_inconsistency` classes/codes, but graph `worldtree.db` quarantine/rebuild is still not implemented. Existing `quarantine_db` helpers are for content/session stores, not graph store.

#### Patch C1 — Graph DB corruption classification

- [x] define a shared `GraphStoreHealthClass` (or equivalent) in core health/readiness code, mapped from existing `IntegrityState`, `GraphExecutionState`, and graph build lifecycle state:
  - [x] `healthy` — existing execution state `fresh` with clean integrity
  - [x] `stale` — existing execution state `stale`; queryable with freshness warning
  - [x] `interrupted_build` — build lifecycle state `building` left from previous run
  - [x] `failed_build` — build lifecycle state `build_failed`
  - [x] `sqlite_corrupt` — `Store::open`, SQLite integrity, or SQLite execution failure proves physical DB corruption
  - [x] `schema_mismatch` — existing schema mismatch detection (`no such table`, missing columns, old migrations)
  - [x] `logical_inconsistency` — DB opens, but graph invariants fail (`foreign_key_check`, dangling edges, structural orphan rows, noncanonical graph path rows)
- [x] split current `corrupt_or_inconsistent_graph_rows` handling into stable machine-readable classes/codes for `sqlite_corrupt` vs `logical_inconsistency`, while preserving any compatibility alias only if needed by callers
- [x] classify evidence sources consistently:
  - [x] `Store::open` errors in `packages/atlas-store-sqlite` callers
  - [x] `PRAGMA integrity_check`
  - [x] `PRAGMA foreign_key_check`
  - [x] orphan-node scan, excluding expected file nodes the same way `db_check` does
  - [x] dangling-edge scan for graph-semantic edge kinds
  - [x] graph build lifecycle state from `graph_build_state`
  - [x] freshness check against changed graph-relevant files
- [x] ensure CLI and MCP use the same classification, `health_class`, `error_code`, message, and suggestions from shared core helpers
- [x] update output schemas and docs for `status`, `doctor`, `db_check`, and `build_or_update_graph`
- [x] add tests for each health class and error-code mapping in core, CLI, and MCP paths

Why:
- makes corruption versus stale data explicit
- avoids treating dangling/orphan graph rows as a generic diagnostics warning
- aligns new policy with existing readiness model instead of adding parallel ad hoc states

Implementation anchors:
- `packages/atlas-core/src/readiness.rs`
- `packages/atlas-core/src/health.rs`
- `packages/atlas-mcp/src/tools/health.rs`
- `packages/atlas-cli/src/commands/maintenance.rs`
- `docs/error_codes.md`

#### Patch C2 — Quarantine and rebuild policy for `worldtree.db`

- [x] define no partial salvage for graph DB corruption unless a future task explicitly adds verified salvage
- [x] define recovery modes:
  - [x] `manual_rebuild_required` — diagnostics report command; operator runs rebuild
  - [x] `auto_quarantine_and_rebuild` — Atlas quarantines DB and rebuilds when command policy allows
  - [x] `block_only` — graph-backed tools refuse answers but do not mutate DB
- [x] define default recovery mode per entry point:
  - [x] `status` / `doctor` / `db_check`: `block_only` diagnostics, no mutation
  - [x] explicit `build` / `update`: `auto_quarantine_and_rebuild` when corruption is detected before graph access
  - [x] graph-backed query/context/analyze tools: `block_only` with rebuild command
- [x] require explicit flag for automatic quarantine outside build/update commands
- [x] implement graph-store quarantine for `.atlas/worldtree.db`; do not reuse content/session quarantine helpers unless they are generalized safely
- [x] quarantine physically corrupt or logically inconsistent `.atlas/worldtree.db` before rebuilding
- [x] use deterministic quarantine path with timestamp and collision-safe suffix, e.g. `.atlas/worldtree.db.quarantine.<UTC>.<n>`
- [x] keep quarantined DB for inspection instead of deleting it
- [x] remove stale WAL/SHM sidecars only as part of the same quarantine operation, preserving them with the quarantined DB when present
- [x] create fresh `worldtree.db` from migrations after quarantine
- [x] run full graph rebuild from repository source after quarantine; do not run incremental update against fresh empty DB
- [x] record quarantine and rebuild result in graph build lifecycle state, including failed rebuild reason
- [x] surface `health_class`, `recovery_mode`, `quarantine_path`, rebuild result, and failure reason in CLI JSON output
- [x] surface same fields in MCP `build_or_update_graph`, `status`, `doctor`, and `db_check` where relevant
- [x] add tests:
  - [x] corrupt SQLite file is quarantined before rebuild
  - [x] logical dangling-edge inconsistency triggers rebuild policy
  - [x] rebuild after quarantine creates usable fresh graph DB
  - [x] failed rebuild leaves graph unavailable with actionable error
  - [x] diagnostics modes report corruption without mutating DB

Why:
- graph data is derived from repo source, so clean rebuild is safer than partial salvage
- quarantine preserves evidence without serving unsafe facts

Implementation anchors:
- `packages/atlas-engine/src/build.rs`
- `packages/atlas-engine/src/update.rs`
- `packages/atlas-mcp/src/tools/context_ops/build.rs`
- `packages/atlas-cli/src/commands/graph/build.rs`
- `packages/atlas-cli/src/commands/graph/update.rs`
- `packages/atlas-store-sqlite`

#### Patch C3 — Block unsafe graph-backed answers

- [x] block graph-backed query/context/traversal/analyze tools when health class is `sqlite_corrupt`, `schema_mismatch`, or `logical_inconsistency`
- [x] wire existing `GraphReadiness::check_tool` policy into MCP graph-backed tools before they call `open_store(db_path)?` and run graph queries
- [x] ensure CLI graph-backed commands keep using the same readiness gate and surface the same class/code fields
- [x] return machine-readable failure with:
  - [x] `error_code`
  - [x] `health_class`
  - [x] `execution_state`
  - [x] `db_path`
  - [x] `quarantine_path` when available
  - [x] `recommended_rebuild_command`
- [x] allow non-graph diagnostics tools to keep working:
  - [x] `status`
  - [x] `doctor`
  - [x] `db_check`
  - [x] `debug_graph` only when DB can open safely
- [x] distinguish stale-but-queryable graph state from corrupt-and-blocked graph state in both payload and `atlas_freshness` / readiness metadata
- [x] document agent behavior: do not answer from graph facts when corrupt/inconsistent; run diagnostics or rebuild instead
- [x] add MCP tests that graph-backed tools fail closed on corrupt/inconsistent DB

Why:
- prevents confident answers from known-bad graph rows
- keeps diagnostics available while blocking unsafe context

Implementation anchors:
- `packages/atlas-core/src/readiness.rs`
- `packages/atlas-mcp/src/tools/graph.rs`
- `packages/atlas-mcp/src/tools/context.rs`
- `packages/atlas-mcp/src/tools/review.rs`
- `packages/atlas-mcp/src/tools/analysis.rs`
- `packages/atlas-cli/src/commands/mod.rs`

#### Patch C completion criteria

- [x] graph DB health classes are explicit, mapped from existing readiness state, and shared by CLI/MCP
- [x] physical SQLite corruption and logical graph inconsistency no longer collapse into one ambiguous class
- [x] corrupt graph execution state maps to block + quarantine + rebuild behavior
- [x] auto rebuild, manual rebuild, and block-only recovery modes are explicit per command/tool
- [x] corrupt or logically inconsistent `worldtree.db` is quarantined before rebuild
- [x] rebuild from source is default policy; partial salvage is explicitly out of scope
- [x] graph-backed tools fail closed when graph facts are corrupt or inconsistent
- [x] diagnostics expose exact reason, health class, quarantine path, and next command
- [x] tests cover physical corruption, logical inconsistency, rebuild success, rebuild failure, diagnostics-no-mutation, and fail-closed query behavior

---

## Additional Backlog

- [x] add canonical `docs/error_codes.md` file and make README, MCP responses, and tests reference that single error-code catalog
- [x] add generated `MCP_TOOLS.md` from tool registry and test/docs check that catches drift from hand-maintained tool tables
- [ ] add build/query/MCP metrics counters and histograms for build duration, parsed file count, parser cache reuse ratio, query latency by mode, and MCP tool call counts
- [x] add informational `cargo-llvm-cov` coverage task and a new github workflow job that reports coverage without gating merge
- [x] add `criterion` bench suites per crate for build, incremental update, query modes, context engine, and history reconstruction workloads
- [x] add CI regression harness for `cargo bench --message-format=json` and store benchmark output as comparable artifact
- [ ] add CI-visible parser cache hit-ratio metric and fail when cache reuse drops below configured threshold
- [ ] add thin LSP shim that maps Atlas query/context/impact/reference flows onto standard LSP requests
- [ ] add documented `budget_policy` block to `.atlas/config.toml` with defaults and `--budget-profile` selection:
  - [ ] document which budget limits are byte-based heuristics versus tokenizer-backed counts
  - [ ] add tokenizer config fields for budget accounting provider, model, and fallback mode
  - [ ] add tokenizer-backed budget accounting for context/review/export paths that already expose `max_tokens`
  - [ ] keep deterministic byte/char fallback when tokenizer is unavailable and surface fallback metadata in JSON output
  - [ ] add tests for tokenizer-backed counts, heuristic fallback, and stable truncation behavior across both modes
- [x] add `proptest` coverage for ranking/trimming, canonical-path normalization, and FTS query escaping

---

## Part XI — Tokenizer-Backed Context Budget Accounting

Goal: replace byte-per-token estimates in context/review payload budgeting with deterministic tokenizer-backed counting using the Rust `tokenizers` crate, while preserving safe byte caps and explicit fallback metadata.

Overview: Atlas currently counts context tokens with `bytes.div_ceil(4)` in `packages/atlas-review/src/context/payload.rs`. This part adds a shared token-counting crate, configuration, runtime integration, output metadata, tests, and validation so CLI/MCP payload truncation can enforce real tokenizer counts without network access.

Rules:
- Use local tokenizer JSON files only; do not add runtime network downloads.
- Keep byte caps as transport and memory safety limits even when tokenizer counts are available.
- Keep additive output changes only; do not remove existing `tokens_estimated` fields in this part.
- Preserve deterministic trimming order across tokenizer and fallback modes.
- Surface fallback metadata in JSON whenever heuristic counting is used because tokenizer loading/counting failed.
- Keep manual setup out of acceptance; every checklist item must be verifiable by tests, generated config, or code.
- Prefer `pub(crate)` APIs unless cross-crate exposure is required.
- Run `cargo fmt --all` after implementation.
- Run clippy quiet mode before completion.

### Phase XI.1 — Token Counting Crate Foundation

Goal: add one reusable crate that wraps `tokenizers` and current heuristic behavior behind a deterministic API.

Overview: create `atlas-token-count` as the only crate that depends directly on external tokenizer machinery. Other crates ask this crate to count tokens and receive count method metadata.

Rules:
- Do not put `tokenizers` dependency in `atlas-core`.
- Do not make model discovery dynamic or network-backed.
- Count exact UTF-8 text passed by callers; do not normalize text before counting.
- Fallback heuristic must match current behavior: `bytes.div_ceil(4)`.
- Errors must include path/config context without leaking payload content.

- [ ] add `packages/atlas-token-count` crate:
  - [ ] create `packages/atlas-token-count/Cargo.toml`:
    - [ ] set package name to `atlas-token-count`
    - [ ] set version to current workspace package version
    - [ ] set edition to `2024`
    - [ ] set `publish = false`
    - [ ] add dependencies:
      - [ ] `anyhow.workspace = true`
      - [ ] `serde.workspace = true`
      - [ ] `tokenizers.workspace = true`
    - [ ] add workspace lint inheritance
  - [ ] create `packages/atlas-token-count/src/lib.rs`:
    - [ ] define `TokenCountMethod` enum:
      - [ ] add `Tokenizer { provider: String, model: Option<String> }`
      - [ ] add `HeuristicBytes { bytes_per_token: usize }`
    - [ ] define `TokenCount` struct:
      - [ ] add `tokens: usize`
      - [ ] add `method: TokenCountMethod`
      - [ ] add `fallback_reason: Option<String>`
    - [ ] define `TokenCounter` enum:
      - [ ] add `Tokenizer { tokenizer: std::sync::Arc<tokenizers::Tokenizer>, provider: String, model: Option<String> }`
      - [ ] add `Heuristic { bytes_per_token: usize }`
    - [ ] implement `TokenCounter::heuristic(bytes_per_token: usize) -> anyhow::Result<Self>`:
      - [ ] reject `bytes_per_token == 0`
      - [ ] store validated value
    - [ ] implement `TokenCounter::from_file(path, provider, model) -> anyhow::Result<Self>`:
      - [ ] load tokenizer with `tokenizers::Tokenizer::from_file`
      - [ ] attach error context containing tokenizer path
      - [ ] store tokenizer in `Arc`
      - [ ] store provider and model metadata
    - [ ] implement `TokenCounter::count_text(&self, text: &str) -> anyhow::Result<TokenCount>`:
      - [ ] for tokenizer mode, call `encode(text, false)`
      - [ ] return encoded token length
      - [ ] for heuristic mode, return `text.len().div_ceil(bytes_per_token)`
      - [ ] return corresponding `TokenCountMethod`
    - [ ] implement `TokenCounter::count_json_bytes(&self, bytes: &[u8]) -> anyhow::Result<TokenCount>`:
      - [ ] convert UTF-8 bytes with error context
      - [ ] delegate to `count_text`
  - [ ] add crate to root workspace members:
    - [ ] append `packages/atlas-token-count` to `[workspace].members`
    - [ ] add `atlas-token-count = { path = "packages/atlas-token-count" }` to `[workspace.dependencies]`
  - [ ] add external `tokenizers` workspace dependency:
    - [ ] add `tokenizers` under `[workspace.dependencies]`
    - [ ] disable default features unless tests prove a required feature is needed
    - [ ] keep version pinned by `Cargo.lock`

- [ ] add token-count unit tests:
  - [ ] create heuristic tests in `packages/atlas-token-count/src/lib.rs` or `tests/heuristic.rs`:
    - [ ] assert empty string counts as `0`
    - [ ] assert one byte counts as `1` with `bytes_per_token = 4`
    - [ ] assert four bytes count as `1`
    - [ ] assert five bytes count as `2`
    - [ ] assert `bytes_per_token = 0` returns error
  - [ ] create tokenizer fixture test:
    - [ ] add minimal valid tokenizer JSON fixture under `packages/atlas-token-count/tests/fixtures/simple-tokenizer.json`
    - [ ] load fixture with `TokenCounter::from_file`
    - [ ] count deterministic sample text
    - [ ] assert `TokenCountMethod::Tokenizer` metadata contains provider and model
  - [ ] create invalid tokenizer test:
    - [ ] pass missing fixture path to `TokenCounter::from_file`
    - [ ] assert error string includes tokenizer path

- [ ] validate Phase XI.1:
  - [ ] run `cargo fmt --all`
  - [ ] run `cargo test --quiet -p atlas-token-count`
  - [ ] run `cargo clippy --workspace --all-targets --quiet`

Phase XI.1 completion criteria:
- [ ] `atlas-token-count` compiles as workspace crate
- [ ] tokenizer-backed counting works from local fixture file
- [ ] heuristic counting exactly preserves current `bytes.div_ceil(4)` behavior
- [ ] invalid tokenizer paths fail with actionable path context
- [ ] `cargo test --quiet -p atlas-token-count` passes

### Phase XI.2 — Configuration and Runtime Token Counter Loading

Goal: make tokenizer accounting configurable from `.atlas/config.toml` with deterministic heuristic fallback.

Overview: add context tokenizer configuration in `atlas-engine`, render it in config templates, and provide a runtime builder that returns a ready `TokenCounter` plus fallback metadata.

Rules:
- Default config must preserve current heuristic behavior.
- Relative tokenizer paths resolve from `.atlas/`, matching existing external config file patterns.
- `fallback = "heuristic"` is default and safe-to-answer.
- `fallback = "fail_closed"` must error before payload truncation when tokenizer loading fails.
- Config validation rejects blank strings and zero byte-per-token values.
- Do not add manual tokenizer download steps.

- [ ] add config model in `packages/atlas-engine/src/config/context.rs`:
  - [ ] add `TokenizerProvider` enum:
    - [ ] support `heuristic`
    - [ ] support `tokenizers`
    - [ ] deserialize from lowercase kebab/snake-friendly strings if existing config style requires it
  - [ ] add `TokenizerFallbackMode` enum:
    - [ ] support `heuristic`
    - [ ] support `fail_closed`
  - [ ] add `ContextTokenizerConfig` struct:
    - [ ] add `provider: TokenizerProvider`
    - [ ] add `model: Option<String>`
    - [ ] add `tokenizer_file: Option<String>`
    - [ ] add `fallback: TokenizerFallbackMode`
    - [ ] add `bytes_per_token: usize`
  - [ ] add `tokenizer: ContextTokenizerConfig` to `ContextConfig`
  - [ ] implement defaults:
    - [ ] set `provider = heuristic`
    - [ ] set `model = None`
    - [ ] set `tokenizer_file = None`
    - [ ] set `fallback = heuristic`
    - [ ] set `bytes_per_token = 4`

- [ ] add validation helpers in `atlas-engine`:
  - [ ] implement `ContextTokenizerConfig::validate(&self) -> anyhow::Result<()>`:
    - [ ] reject `bytes_per_token == 0`
    - [ ] reject blank `model` when `Some`
    - [ ] reject blank `tokenizer_file` when `Some`
    - [ ] require `tokenizer_file` when `provider = tokenizers`
    - [ ] reject `tokenizer_file` when `provider = heuristic` if existing config policy rejects ignored fields
  - [ ] call tokenizer validation from `Config::load` validation path
  - [ ] include config key names in validation errors:
    - [ ] `context.tokenizer.bytes_per_token`
    - [ ] `context.tokenizer.model`
    - [ ] `context.tokenizer.tokenizer_file`

- [ ] add runtime builder in `atlas-engine`:
  - [ ] add `Config::token_counter(&self, atlas_dir: &Utf8Path) -> anyhow::Result<TokenCounterLoadResult>`
  - [ ] define `TokenCounterLoadResult`:
    - [ ] add `counter: atlas_token_count::TokenCounter`
    - [ ] add `fallback_used: bool`
    - [ ] add `fallback_reason: Option<String>`
  - [ ] implement heuristic provider path:
    - [ ] return `TokenCounter::heuristic(bytes_per_token)`
    - [ ] set `fallback_used = false`
  - [ ] implement tokenizers provider success path:
    - [ ] resolve relative `tokenizer_file` from `atlas_dir`
    - [ ] load tokenizer via `TokenCounter::from_file`
    - [ ] set provider string to `tokenizers`
    - [ ] preserve configured model metadata
  - [ ] implement tokenizers provider load failure with heuristic fallback:
    - [ ] when `fallback = heuristic`, return heuristic counter
    - [ ] set `fallback_used = true`
    - [ ] set fallback reason to load error summary without payload text
  - [ ] implement tokenizers provider load failure with fail-closed:
    - [ ] when `fallback = fail_closed`, return error
    - [ ] include `context.tokenizer.tokenizer_file` and resolved path in error context

- [ ] update config template rendering:
  - [ ] add `[context.tokenizer]` block in `packages/atlas-engine/src/config/template.rs`
  - [ ] render default profile with active heuristic values
  - [ ] include commented tokenizer-backed example values without activating nonexistent files
  - [ ] ensure generated template never points to missing active tokenizer file by default

- [ ] update doctor/runtime config output:
  - [ ] include tokenizer provider in `packages/atlas-cli/src/commands/maintenance.rs` runtime JSON
  - [ ] include tokenizer model when configured
  - [ ] include fallback mode
  - [ ] include bytes-per-token
  - [ ] do not include resolved tokenizer contents

- [ ] add config tests:
  - [ ] load default config and assert heuristic tokenizer defaults
  - [ ] load config with `provider = "tokenizers"` and valid relative `tokenizer_file`
  - [ ] assert relative `tokenizer_file` resolves from `.atlas/`
  - [ ] reject missing `tokenizer_file` when provider is `tokenizers`
  - [ ] reject blank `tokenizer_file`
  - [ ] reject blank `model`
  - [ ] reject `bytes_per_token = 0`
  - [ ] assert missing tokenizer file falls back when fallback mode is heuristic
  - [ ] assert missing tokenizer file errors when fallback mode is fail-closed
  - [ ] assert config template includes `[context.tokenizer]`

- [ ] validate Phase XI.2:
  - [ ] run `cargo fmt --all`
  - [ ] run `cargo test --quiet -p atlas-engine config::tests::load_default_context_tokenizer_config`
  - [ ] run `cargo test --quiet -p atlas-engine config::tests::load_tokenizers_provider_with_relative_file`
  - [ ] run `cargo test --quiet -p atlas-engine config::tests::tokenizer_provider_missing_file_falls_back_to_heuristic`
  - [ ] run `cargo test --quiet -p atlas-engine config::tests::tokenizer_provider_missing_file_fail_closed_errors`
  - [ ] run `cargo clippy --workspace --all-targets --quiet`

Phase XI.2 completion criteria:
- [ ] `.atlas/config.toml` supports `[context.tokenizer]`
- [ ] default behavior remains heuristic with four bytes per token
- [ ] tokenizer files resolve relative to `.atlas/`
- [ ] fallback and fail-closed modes are covered by automated tests
- [ ] runtime config output exposes tokenizer settings without exposing tokenizer contents

### Phase XI.3 — Review Context Payload Integration

Goal: enforce context token budgets with `TokenCounter` instead of byte-derived token estimates.

Overview: update `atlas-review` payload budgeting to measure serialized context payloads through the configured counter, while keeping byte caps and current item trimming priority.

Rules:
- Measure the same serialized JSON payload that CLI/MCP emits after removing payload metadata from the clone.
- Keep byte-limit trimming active even when tokenizer count is below token limit.
- Preserve existing direct-target retention behavior.
- Do not reorder trimming candidates.
- Do not add token counting to graph traversal or ranking in this phase.
- Do not panic on tokenizer errors; use loaded fallback counter or return typed build error from fail-closed config path.

- [ ] add dependency to `atlas-review`:
  - [ ] add `atlas-token-count.workspace = true` to `packages/atlas-review/Cargo.toml`

- [ ] thread token counter into `ContextEngine`:
  - [ ] add token counter field to context engine state with heuristic default
  - [ ] add builder method `ContextEngine::with_token_counter(counter: TokenCounter) -> Self`
  - [ ] keep existing constructors behavior-compatible by defaulting to heuristic `bytes_per_token = 4`
  - [ ] update engine call sites to pass configured counter from `atlas-engine::Config::token_counter`
  - [ ] update CLI context command path to load token counter from config
  - [ ] update MCP context/review tool path to load token counter from config
  - [ ] update session/review refresh action path to load token counter from config

- [ ] replace heuristic-only measurement in `packages/atlas-review/src/context/payload.rs`:
  - [ ] replace `estimate_tokens(bytes: usize)` helper with serialized-payload measurement helper
  - [ ] add `PayloadMeasurement` struct:
    - [ ] `bytes: usize`
    - [ ] `tokens: usize`
    - [ ] `method: TokenCountMethod`
    - [ ] `fallback_reason: Option<String>`
  - [ ] update `context_bytes` helper to return serialized bytes or shared measurement input
  - [ ] update `trim_context_payload` signature:
    - [ ] accept `counter: &TokenCounter`
    - [ ] count current serialized payload each loop
    - [ ] break only when `bytes <= byte_limit` and `tokens <= token_limit`
    - [ ] return final `PayloadMeasurement`
  - [ ] update `apply_payload_budgets`:
    - [ ] accept `counter: &TokenCounter`
    - [ ] compute requested payload bytes and requested token count before trimming
    - [ ] compute final emitted payload bytes and final token count after trimming
    - [ ] populate `tokens_estimated` with tokenizer-backed count for compatibility
    - [ ] preserve `omitted_byte_count` semantics as requested bytes minus emitted bytes
  - [ ] update source mix calculation:
    - [ ] accept `counter: &TokenCounter`
    - [ ] count serialized graph context section with tokenizer
    - [ ] count serialized content assets section with tokenizer
    - [ ] count serialized saved artifacts section with tokenizer
    - [ ] keep item included/dropped counts unchanged
    - [ ] use heuristic fallback only when counter itself is heuristic

- [ ] keep byte-derived byte limit calculation safe:
  - [ ] remove token-to-byte limit derivation as primary enforcement
  - [ ] set effective byte limit to policy `context_payload_bytes.default_limit`
  - [ ] rely on tokenizer count for token budget and byte cap for payload size
  - [ ] keep per-request `token_budget` capped by policy default and max limit exactly as current behavior

- [ ] preserve existing trimming helpers:
  - [ ] keep `trim_file_excerpt_bytes` behavior unchanged
  - [ ] keep `trim_saved_context_bytes` behavior unchanged
  - [ ] keep `trim_review_source_bytes` behavior unchanged
  - [ ] keep `trim_one_payload_unit` candidate priority unchanged
  - [ ] keep direct target retention tests passing

- [ ] update call sites and tests that call `apply_payload_budgets` directly:
  - [ ] pass `&TokenCounter::heuristic(4).expect(...)` in existing unit tests
  - [ ] avoid global/static token counter state in tests
  - [ ] keep per-test local counter values

- [ ] validate Phase XI.3:
  - [ ] run `cargo fmt --all`
  - [ ] run `cargo test --quiet -p atlas-review context::tests::budget`
  - [ ] run `cargo test --quiet -p atlas-cli cli_quality_gates`
  - [ ] run `cargo clippy --workspace --all-targets --quiet`

Phase XI.3 completion criteria:
- [ ] context/review payload token budgets use `TokenCounter`
- [ ] default review behavior remains compatible through heuristic counter
- [ ] byte caps still enforce payload safety independently of token counts
- [ ] direct target retention and truncation metadata tests pass
- [ ] CLI, MCP, and session refresh paths use configured token accounting

### Phase XI.4 — Token Accounting Metadata in CLI and MCP Outputs

Goal: expose how tokens were counted so agents can tell tokenizer-backed counts from heuristic fallback.

Overview: add optional metadata next to existing payload truncation fields. Keep current `tokens_estimated` field stable and populate it with the actual count from whichever method was used.

Rules:
- Add fields only; do not remove or rename existing JSON keys.
- Serialize metadata only when payload trimming metadata exists.
- Do not include tokenizer file contents or absolute secret-bearing paths in MCP payloads.
- Keep CLI text concise; JSON carries full structured metadata.
- MCP schema additions must remain compatible with current structured content response shapes.

- [ ] add core model metadata in `packages/atlas-core/src/model/context.rs`:
  - [ ] define `TokenAccountingMeta`:
    - [ ] `provider: String`
    - [ ] `model: Option<String>` with `skip_serializing_if`
    - [ ] `fallback_used: bool`
    - [ ] `fallback_reason: Option<String>` with `skip_serializing_if`
    - [ ] `bytes_per_token: Option<usize>` with `skip_serializing_if`
  - [ ] add `token_accounting: Option<TokenAccountingMeta>` to `PayloadTruncationMeta`
  - [ ] add serde skip for `None`
  - [ ] keep `tokens_estimated` unchanged
  - [ ] update doc comments to say `tokens_estimated` is compatibility name for counted tokens

- [ ] populate token accounting metadata in `atlas-review`:
  - [ ] map `TokenCountMethod::Tokenizer` to provider/model metadata
  - [ ] map `TokenCountMethod::HeuristicBytes` to provider `heuristic`
  - [ ] set `bytes_per_token` for heuristic counts
  - [ ] set `fallback_used = true` when load result indicated fallback
  - [ ] copy fallback reason from load result or count result
  - [ ] ensure metadata appears when caller token budget override applies but no payload units dropped

- [ ] update CLI text output for review/context truncation:
  - [ ] print token provider when payload metadata exists
  - [ ] print model when present
  - [ ] print fallback marker when fallback was used
  - [ ] preserve existing truncation lines and ordering where possible

- [ ] update MCP schemas/manual-generated docs through registry-backed tests:
  - [ ] add `token_accounting` metadata to relevant response schema if schema is explicit
  - [ ] update manual docs tests to assert payload metadata includes token accounting fields where applicable
  - [ ] keep `structuredContent` JSON source of truth

- [ ] add metadata tests:
  - [ ] assert heuristic mode emits `token_accounting.provider = "heuristic"`
  - [ ] assert heuristic mode emits `bytes_per_token = 4`
  - [ ] assert tokenizer mode emits `provider = "tokenizers"`
  - [ ] assert tokenizer mode emits configured model
  - [ ] assert fallback mode emits `fallback_used = true`
  - [ ] assert fallback mode emits non-empty fallback reason
  - [ ] assert serialized old fields still include `tokens_estimated`

- [ ] validate Phase XI.4:
  - [ ] run `cargo fmt --all`
  - [ ] run `cargo test --quiet -p atlas-core context`
  - [ ] run `cargo test --quiet -p atlas-review context::tests::budget`
  - [ ] run `cargo test --quiet -p atlas-mcp`
  - [ ] run `cargo clippy --workspace --all-targets --quiet`

Phase XI.4 completion criteria:
- [ ] JSON payload truncation metadata reports tokenizer provider/method
- [ ] fallback use is visible to CLI/MCP callers
- [ ] `tokens_estimated` remains present for compatibility
- [ ] MCP schema/manual tests pass with additive metadata fields

### Phase XI.5 — Deterministic Tokenizer Truncation Tests and Quality Gates

Goal: prove tokenizer-backed budgeting is deterministic, bounded, and behavior-compatible across CLI/MCP/review paths.

Overview: add fixtures and tests that force different token counts than byte heuristic so regressions cannot silently return to byte-only accounting.

Rules:
- Tests must not depend on real network, global env, or shared mutable tokenizer cache.
- Tests must use per-test temp directories or committed fixtures.
- Avoid fixed sleeps and retries.
- Do not use shell-level test retries.
- Keep tests quiet and targeted.

- [ ] add tokenizer-sensitive fixture coverage:
  - [ ] create committed tokenizer fixture that produces known counts for repeated punctuation/text samples
  - [ ] add review payload sample where tokenizer count differs from `bytes.div_ceil(4)`
  - [ ] assert trimming happens due to tokenizer token count while byte cap remains high
  - [ ] assert no trimming happens when tokenizer token count is below cap and byte cap is high

- [ ] add truncation-order regression tests:
  - [ ] seed result with saved context, workflow metadata, ambiguity metadata, files, edges, and nodes
  - [ ] apply tight token budget
  - [ ] assert saved context drops before graph files
  - [ ] assert files drop before direct target nodes
  - [ ] assert direct target nodes remain when any removable lower-priority item exists
  - [ ] assert source mix item counts reflect dropped sections

- [ ] add CLI integration tests:
  - [ ] create temp repo config with `[context.tokenizer] provider = "tokenizers"`
  - [ ] place tokenizer fixture under temp `.atlas/tokenizers/`
  - [ ] run focused CLI context/review command with JSON output
  - [ ] assert `truncation.payload.token_accounting.provider = "tokenizers"`
  - [ ] assert `tokens_estimated` equals fixture-backed expected count

- [ ] add fallback integration tests:
  - [ ] create temp config with missing tokenizer file and `fallback = "heuristic"`
  - [ ] run focused context/review command
  - [ ] assert command succeeds
  - [ ] assert `fallback_used = true`
  - [ ] assert fallback reason is non-empty
  - [ ] create temp config with missing tokenizer file and `fallback = "fail_closed"`
  - [ ] run focused context/review command
  - [ ] assert command fails with error naming `context.tokenizer.tokenizer_file`

- [ ] add MCP parity tests:
  - [ ] run MCP `get_context` path with tokenizer config fixture
  - [ ] assert structured content contains token accounting metadata
  - [ ] assert CLI and MCP JSON agree on token provider and fallback flag for same fixture input

- [ ] validate Phase XI.5:
  - [ ] run `cargo fmt --all`
  - [ ] run `cargo test --quiet -p atlas-review tokenizer_budget`
  - [ ] run `cargo test --quiet -p atlas-cli --test cli_quality_gates tokenizer_budget`
  - [ ] run `cargo test --quiet -p atlas-mcp tokenizer_budget`
  - [ ] run `cargo clippy --workspace --all-targets --quiet`

Phase XI.5 completion criteria:
- [ ] tokenizer-specific tests fail if code returns to byte-only token counting
- [ ] truncation order remains deterministic under tokenizer-backed counting
- [ ] CLI JSON exposes tokenizer metadata and fallback metadata
- [ ] MCP structured content matches CLI token accounting behavior
- [ ] fail-closed tokenizer config fails with actionable config-key error

### Phase XI.6 — Documentation, Issue Closure, and Workspace Validation

Goal: finalize tokenizer-backed budget accounting with generated docs, config examples, and workspace-wide validation.

Overview: update checked-in docs/templates and mark this backlog as complete only after automated gates prove runtime, config, and output behavior.

Rules:
- Do not add manual verification checklist items.
- Do not mark completion until formatter, clippy, targeted tests, and workspace summary pass.
- Documentation examples must be generated or test-covered when possible.
- Keep docs clear that tokenizer JSON files are local inputs and not downloaded automatically.

- [ ] update generated/config documentation:
  - [ ] update config template snapshot tests for `[context.tokenizer]`
  - [ ] update docs that describe budget limits:
    - [ ] distinguish byte caps from tokenizer-backed token caps
    - [ ] describe default heuristic mode
    - [ ] describe local tokenizer file mode
    - [ ] describe fallback metadata fields
    - [ ] describe fail-closed behavior
  - [ ] update any generated MCP tool docs that include context payload metadata
  - [ ] add docs drift tests when existing docs pipeline supports them

- [ ] update `ISSUES.md` tracking:
  - [ ] mark the Additional Backlog token-budget item as complete only after all Phase XI criteria pass
  - [ ] mark all Part XI checklist items complete after validation commands pass

- [ ] run final validation:
  - [ ] run `cargo fmt --all`
  - [ ] run `cargo clippy --workspace --all-targets --quiet`
  - [ ] run `cargo test --quiet -p atlas-token-count`
  - [ ] run `cargo test --quiet -p atlas-engine config`
  - [ ] run `cargo test --quiet -p atlas-review context::tests::budget`
  - [ ] run `cargo test --quiet -p atlas-cli --test cli_quality_gates`
  - [ ] run `cargo test --quiet -p atlas-mcp`
  - [ ] run `./scripts/test-workspace-summary.sh`

Phase XI.6 completion criteria:
- [ ] config templates and docs describe tokenizer-backed budget accounting
- [ ] existing backlog token-budget item is marked complete
- [ ] all targeted tokenizer/config/review/CLI/MCP tests pass
- [ ] `cargo clippy --workspace --all-targets --quiet` passes
- [ ] `./scripts/test-workspace-summary.sh` passes

Part XI completion criteria:
- [ ] Atlas supports local-file `tokenizers` counting for context/review payload budgets
- [ ] Atlas defaults remain deterministic and compatible through four-bytes-per-token heuristic mode
- [ ] CLI/MCP outputs expose token accounting method and fallback metadata
- [ ] byte caps remain enforced independently from token caps
- [ ] tokenizer-backed, heuristic fallback, and fail-closed modes are covered by automated tests
- [ ] docs/templates describe byte-based and tokenizer-backed budget limits
