//! Static resource and resource-template descriptors.

use serde_json::{Value, json};

use crate::descriptors::{
    ResourceDescriptor, ResourceTemplateDescriptor, human_title, validate_descriptor_name,
};

pub(crate) fn resource_descriptors() -> Vec<ResourceDescriptor> {
    let mut resources = vec![
        build_resource_descriptor(
            "atlas://docs/index",
            "docs_index",
            "Atlas docs index with resource usage, tool discovery flow, and per-tool docs links.",
            "text/markdown",
            "content",
        ),
        build_resource_descriptor(
            "atlas://graph/provenance",
            "graph_provenance",
            "Atlas graph provenance metadata for current repo and DB.",
            "application/json",
            "health",
        ),
        build_resource_descriptor(
            "atlas://health/status",
            "health_status",
            "Compact Atlas graph health summary for current repo.",
            "application/json",
            "health",
        ),
    ];
    resources.sort_by(|left, right| left.uri.cmp(&right.uri));
    for resource in &resources {
        validate_descriptor_name(&resource.name).expect("resource name must satisfy MCP guidance");
    }
    resources
}

fn build_resource_descriptor(
    uri: &str,
    name: &str,
    description: &str,
    mime_type: &str,
    category: &str,
) -> ResourceDescriptor {
    let mut descriptor = ResourceDescriptor::new(uri, name)
        .with_title(human_title(name))
        .with_description(description)
        .with_mime_type(mime_type);
    if let Some(meta) = crate::rmcp_types::meta_object_from_value(
        crate::descriptors::descriptor_meta("resource", category),
    )
    .expect("resource descriptor meta must be object")
    {
        descriptor = descriptor.with_meta(meta);
    }
    descriptor
}

fn build_resource_template_descriptor(
    uri_template: &str,
    name: &str,
    description: &str,
    mime_type: &str,
    meta: Value,
) -> ResourceTemplateDescriptor {
    let mut descriptor = ResourceTemplateDescriptor::new(uri_template, name)
        .with_title(human_title(name))
        .with_description(description)
        .with_mime_type(mime_type);
    if let Some(meta) = crate::rmcp_types::meta_object_from_value(meta)
        .expect("resource template descriptor meta must be object")
    {
        descriptor = descriptor.with_meta(meta);
    }
    descriptor
}

pub(crate) fn resource_template_descriptors() -> Vec<ResourceTemplateDescriptor> {
    let mut templates = vec![
        build_resource_template_descriptor(
            "atlas://docs/{file}#{heading}",
            "docs_section",
            "Read Markdown docs section by repo-relative file and heading path/slug.",
            "text/markdown",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "content",
                "atlas:variables": [
                    {"name": "file", "description": "Repo-relative Markdown path."},
                    {"name": "heading", "description": "Heading path or slug."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://saved-context/{source_id}",
            "saved_context",
            "Read saved artifact content by source_id.",
            "text/plain",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "memory",
                "atlas:variables": [
                    {"name": "source_id", "description": "Saved artifact source identifier."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://tool-docs/{name}",
            "tool_docs",
            "Read generated Markdown docs, examples, and usage for one exported MCP tool by exact name.",
            "text/markdown",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "content",
                "atlas:variables": [
                    {"name": "name", "description": "Exact exported MCP tool name."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://prompt-docs/{name}",
            "prompt_docs",
            "Read generated Markdown docs and default prompt body for one exported MCP prompt by exact name.",
            "text/markdown",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "content",
                "atlas:variables": [
                    {"name": "name", "description": "Exact exported MCP prompt name."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://repo/{repo_id}/tree/{dir}",
            "repo_tree",
            "List immediate children (directories, files, L0 overviews) of one canonical repo directory. Supports ?level=l0|l1|l2.",
            "application/json",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "graph",
                "atlas:variables": [
                    {"name": "repo_id", "description": "Stable repo id from repo_registry."},
                    {"name": "dir", "description": "Percent-encoded canonical repo-relative directory path; use '.' for the repo root."},
                    {"name": "level", "description": "Optional overview level: l0, l1, or l2. Defaults to l1."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://repo/{repo_id}/file/{path}",
            "repo_file",
            "Read one repo file as an overview projection. Supports ?level=l0|l1|l2.",
            "application/json",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "graph",
                "atlas:variables": [
                    {"name": "repo_id", "description": "Stable repo id from repo_registry."},
                    {"name": "path", "description": "Percent-encoded canonical repo-relative file path."},
                    {"name": "level", "description": "Optional overview level: l0, l1, or l2. Defaults to l1."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://repo/{repo_id}/symbol/{qualified_name}",
            "repo_symbol",
            "Read one graph symbol projection. Supports ?level=l0|l1|l2.",
            "application/json",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "graph",
                "atlas:variables": [
                    {"name": "repo_id", "description": "Stable repo id from repo_registry."},
                    {"name": "qualified_name", "description": "Percent-encoded fully qualified symbol name."},
                    {"name": "level", "description": "Optional overview level: l0, l1, or l2. Defaults to l1."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://memories/topic/{topic}",
            "memory_topic",
            "Read the memory timeline for one topic.",
            "application/json",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "memory",
                "atlas:variables": [
                    {"name": "topic", "description": "Memory topic label."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://memories/id/{memory_id}",
            "memory_id",
            "Read one memory record by id.",
            "application/json",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "memory",
                "atlas:variables": [
                    {"name": "memory_id", "description": "Memory record id."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://memoir/{graph}/{concept}",
            "memoir_concept",
            "Read one memoir concept plus its 1-hop relations.",
            "application/json",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "memory",
                "atlas:variables": [
                    {"name": "graph", "description": "Memoir graph name or id."},
                    {"name": "concept", "description": "Memoir concept name."}
                ]
            }),
        ),
        build_resource_template_descriptor(
            "atlas://artifacts/{source_id}/{chunk_index}",
            "artifact_chunk",
            "Read one saved artifact chunk; level defaults to l2 (full chunk text).",
            "text/plain",
            json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "content",
                "atlas:variables": [
                    {"name": "source_id", "description": "Saved artifact source identifier."},
                    {"name": "chunk_index", "description": "Zero-based chunk index."},
                    {"name": "level", "description": "Optional level: l0 (preview), l1 (bounded), l2 (full). Defaults to l2."}
                ]
            }),
        ),
    ];
    templates.sort_by(|left, right| left.uri_template.cmp(&right.uri_template));
    for template in &templates {
        validate_descriptor_name(&template.name).expect("template name must satisfy MCP guidance");
    }
    templates
}
