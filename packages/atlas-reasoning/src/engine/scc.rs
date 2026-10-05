//! Deterministic Tarjan strongly-connected-components over a string adjacency
//! map.
//!
//! Shared by module-cycle analysis (`architecture.rs`) and risk cycle context
//! (`risk.rs`), which previously carried identical private copies.

use std::collections::{BTreeMap, HashMap, HashSet};

/// Return every strongly connected component of `adjacency`.
///
/// Components and their members are sorted so results are deterministic for a
/// given adjacency map.
pub(crate) fn strongly_connected_components(
    adjacency: &BTreeMap<String, Vec<String>>,
) -> Vec<Vec<String>> {
    struct TarjanState {
        index: usize,
        stack: Vec<String>,
        on_stack: HashSet<String>,
        indices: HashMap<String, usize>,
        lowlink: HashMap<String, usize>,
        components: Vec<Vec<String>>,
    }

    fn strong_connect(
        node: &str,
        adjacency: &BTreeMap<String, Vec<String>>,
        state: &mut TarjanState,
    ) {
        let current_index = state.index;
        state.indices.insert(node.to_owned(), current_index);
        state.lowlink.insert(node.to_owned(), current_index);
        state.index += 1;
        state.stack.push(node.to_owned());
        state.on_stack.insert(node.to_owned());

        let neighbors = adjacency.get(node).cloned().unwrap_or_default();
        for neighbor in neighbors {
            if !state.indices.contains_key(&neighbor) {
                strong_connect(&neighbor, adjacency, state);
                let low_neighbor = state.lowlink[&neighbor];
                let low_node = state.lowlink[node];
                state
                    .lowlink
                    .insert(node.to_owned(), low_node.min(low_neighbor));
            } else if state.on_stack.contains(&neighbor) {
                let neighbor_index = state.indices[&neighbor];
                let low_node = state.lowlink[node];
                state
                    .lowlink
                    .insert(node.to_owned(), low_node.min(neighbor_index));
            }
        }

        if state.lowlink[node] == state.indices[node] {
            let mut component = Vec::new();
            while let Some(item) = state.stack.pop() {
                state.on_stack.remove(&item);
                component.push(item.clone());
                if item == node {
                    break;
                }
            }
            component.sort();
            state.components.push(component);
        }
    }

    let mut state = TarjanState {
        index: 0,
        stack: Vec::new(),
        on_stack: HashSet::new(),
        indices: HashMap::new(),
        lowlink: HashMap::new(),
        components: Vec::new(),
    };

    for node in adjacency.keys() {
        if !state.indices.contains_key(node) {
            strong_connect(node, adjacency, &mut state);
        }
    }

    state
        .components
        .sort_by(|left, right| left.first().cmp(&right.first()));
    state.components
}

#[cfg(test)]
mod tests {
    use super::strongly_connected_components;
    use std::collections::BTreeMap;

    #[test]
    fn finds_cycle_and_isolated_node() {
        let adjacency = BTreeMap::from([
            ("a".to_owned(), vec!["b".to_owned()]),
            ("b".to_owned(), vec!["a".to_owned()]),
            ("c".to_owned(), Vec::new()),
        ]);
        let components = strongly_connected_components(&adjacency);
        assert_eq!(
            components,
            vec![vec!["a".to_owned(), "b".to_owned()], vec!["c".to_owned()],]
        );
    }

    #[test]
    fn target_only_nodes_become_singleton_components() {
        // The algorithm walks edge targets even when they have no adjacency
        // entry, so unknown targets surface as singleton components.  This
        // matches the behavior both former copies shared.
        let adjacency = BTreeMap::from([("a".to_owned(), vec!["missing".to_owned()])]);
        let components = strongly_connected_components(&adjacency);
        assert_eq!(
            components,
            vec![vec!["a".to_owned()], vec!["missing".to_owned()]]
        );
    }
}
