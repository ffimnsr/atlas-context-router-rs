use super::*;

pub(super) fn function_value_node<'a>(node: TsNode<'a>, source: &[u8]) -> Option<TsNode<'a>> {
    match node.kind() {
        "function" | "function_expression" | "arrow_function" => Some(node),
        "call_expression" => {
            let function = node.child_by_field_name("function")?;
            if !matches!(
                node_text(function, source),
                "memo" | "forwardRef" | "React.memo"
            ) {
                return None;
            }
            first_function_argument(node)
        }
        _ => None,
    }
}

pub(super) fn first_function_argument(node: TsNode<'_>) -> Option<TsNode<'_>> {
    let args = node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    args.children(&mut cursor).find(|child| {
        matches!(
            child.kind(),
            "function" | "function_expression" | "arrow_function"
        )
    })
}

pub(super) fn is_identifier_like(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|c| c == '_' || c == '$' || c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c == '_' || c == '$' || c.is_ascii_alphanumeric())
}

pub(super) fn node_key(node: TsNode<'_>) -> JsTsNodeKey {
    JsTsNodeKey {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}

pub(super) fn has_captured_variable_declarator(node: TsNode<'_>, facts: &JsTsSyntaxFacts) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| {
        child.kind() == "variable_declarator" && facts.variables.contains(&node_key(child))
    })
}

// ---------------------------------------------------------------------------
// Same-file call resolution (JavaScript / TypeScript)
// ---------------------------------------------------------------------------

fn import_binding_json(local: &str, imported: Option<&str>, kind: &str) -> serde_json::Value {
    serde_json::json!({
        "local": local,
        "imported": imported,
        "kind": kind,
    })
}

pub(super) fn parse_js_import_bindings(statement: &str) -> Vec<serde_json::Value> {
    let trimmed = statement.trim().trim_end_matches(';').trim();
    let Some(rest) = trimmed.strip_prefix("import") else {
        return Vec::new();
    };
    let rest = rest.trim();
    if rest.starts_with('"') || rest.starts_with('\'') || rest.starts_with('`') {
        return Vec::new();
    }
    let clause = rest
        .split_once(" from ")
        .map(|(head, _)| head)
        .unwrap_or(rest);
    let mut bindings = Vec::new();
    let mut remaining = clause.trim();

    if let Some((default_part, tail)) = remaining.split_once(',') {
        let default_local = default_part.trim();
        if !default_local.is_empty() {
            bindings.push(import_binding_json(
                default_local,
                Some("default"),
                "default",
            ));
        }
        remaining = tail.trim();
    }

    if remaining.starts_with("* as ") {
        let local = remaining.trim_start_matches("* as ").trim();
        if !local.is_empty() {
            bindings.push(import_binding_json(local, None, "namespace"));
        }
        return bindings;
    }

    if remaining.starts_with('{') && remaining.ends_with('}') {
        let inner = &remaining[1..remaining.len().saturating_sub(1)];
        for part in inner.split(',') {
            let entry = part.trim();
            if entry.is_empty() {
                continue;
            }
            let (imported, local) = entry
                .split_once(" as ")
                .map(|(imported, local)| (imported.trim(), local.trim()))
                .unwrap_or((entry, entry));
            bindings.push(import_binding_json(local, Some(imported), "named"));
        }
        return bindings;
    }

    if !remaining.is_empty() {
        bindings.push(import_binding_json(remaining, Some("default"), "default"));
    }

    bindings
}

pub(super) fn resolve_js_calls(
    root: TsNode<'_>,
    source: &[u8],
    rel_path: &str,
    facts: &JsTsSyntaxFacts,
    nodes: &[Node],
) -> Vec<Edge> {
    let callables = callable_qn_map(nodes, true);
    let mut edges = Vec::new();
    let mut scope: Vec<String> = Vec::new();
    walk_js_calls(
        root, source, rel_path, facts, &callables, &mut scope, &mut edges,
    );
    edges
}

pub(super) fn walk_js_calls<'a>(
    node: TsNode<'a>,
    source: &[u8],
    rel_path: &str,
    facts: &JsTsSyntaxFacts,
    callables: &HashMap<String, String>,
    scope: &mut Vec<String>,
    edges: &mut Vec<Edge>,
) {
    let kind = node.kind();
    let key = node_key(node);
    let is_function_scope = facts.functions.contains(&key) || facts.methods.contains(&key);

    if is_function_scope {
        let pushed = if let Some(name_node) = node.child_by_field_name("name") {
            let name = node_text(name_node, source);
            if let Some(qn) = callables.get(name) {
                scope.push(qn.clone());
                true
            } else {
                false
            }
        } else {
            false
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk_js_calls(child, source, rel_path, facts, callables, scope, edges);
        }
        if pushed {
            scope.pop();
        }
        return;
    }

    if facts.variables.contains(&key)
        && let (Some(name_node), Some(value_node)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        )
        && function_value_node(value_node, source).is_some()
    {
        let name = node_text(name_node, source);
        let pushed = if let Some(qn) = callables.get(name) {
            scope.push(qn.clone());
            true
        } else {
            false
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk_js_calls(child, source, rel_path, facts, callables, scope, edges);
        }
        if pushed {
            scope.pop();
        }
        return;
    }

    if facts.calls.contains(&key)
        && kind == "call_expression"
        && let Some(caller_qn) = scope.last().cloned()
    {
        let called = node
            .child_by_field_name("function")
            .and_then(|f| call_target(f, source, "member_expression", "property", "object"));
        if let Some((text, name, receiver)) = called
            && !is_self_call(&caller_qn, &name, receiver.as_deref())
        {
            if let Some(callee_qn) = callables.get(&name)
                && *callee_qn != caller_qn
            {
                edges.push(call_edge(
                    &caller_qn,
                    callee_qn,
                    rel_path,
                    start_line(node),
                    &text,
                    receiver.as_deref(),
                    true,
                ));
            } else if !text.is_empty() {
                edges.push(call_edge(
                    &caller_qn,
                    &text,
                    rel_path,
                    start_line(node),
                    &text,
                    receiver.as_deref(),
                    false,
                ));
            }
        }
    }

    if matches!(kind, "jsx_opening_element" | "jsx_self_closing_element")
        && let Some(caller_qn) = scope.last().cloned()
        && let Some((text, name, receiver)) = jsx_call_target(node, source)
    {
        if let Some(callee_qn) = callables.get(&name)
            && *callee_qn != caller_qn
        {
            edges.push(call_edge(
                &caller_qn,
                callee_qn,
                rel_path,
                start_line(node),
                &text,
                receiver.as_deref(),
                true,
            ));
        } else if !text.is_empty() {
            edges.push(call_edge(
                &caller_qn,
                &text,
                rel_path,
                start_line(node),
                &text,
                receiver.as_deref(),
                false,
            ));
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_js_calls(child, source, rel_path, facts, callables, scope, edges);
    }
}

fn jsx_call_target(node: TsNode<'_>, source: &[u8]) -> Option<(String, String, Option<String>)> {
    let name_node = node.child_by_field_name("name").or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor).find(|child| {
            matches!(
                child.kind(),
                "identifier" | "nested_identifier" | "member_expression" | "jsx_identifier"
            )
        })
    })?;
    let text = node_text(name_node, source).to_owned();
    if text.is_empty() || text.as_bytes().first().is_some_and(u8::is_ascii_lowercase) {
        return None;
    }
    if let Some((receiver, name)) = text.rsplit_once('.') {
        return Some((text.clone(), name.to_owned(), Some(receiver.to_owned())));
    }
    Some((text.clone(), text, None))
}

pub(super) fn is_self_call(caller_qn: &str, callee_name: &str, receiver: Option<&str>) -> bool {
    if receiver.is_some() {
        return false;
    }
    caller_simple_name(caller_qn) == callee_name
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
