use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn strongly_connected_components(
    graph: &BTreeMap<String, BTreeSet<String>>,
    include_self_loops: bool,
) -> Vec<Vec<String>> {
    struct TarjanState {
        index: usize,
        stack: Vec<String>,
        on_stack: BTreeSet<String>,
        indices: BTreeMap<String, usize>,
        lowlinks: BTreeMap<String, usize>,
        components: Vec<Vec<String>>,
        include_self_loops: bool,
    }

    fn strong_connect(
        node: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        state: &mut TarjanState,
    ) {
        let index = state.index;
        state.indices.insert(node.to_owned(), index);
        state.lowlinks.insert(node.to_owned(), index);
        state.index += 1;
        state.stack.push(node.to_owned());
        state.on_stack.insert(node.to_owned());

        for target in graph
            .get(node)
            .into_iter()
            .flat_map(|targets| targets.iter())
        {
            if !state.indices.contains_key(target) {
                strong_connect(target, graph, state);
                let target_low = *state.lowlinks.get(target).expect("lowlink");
                let lowlink = state.lowlinks.get_mut(node).expect("node lowlink");
                *lowlink = (*lowlink).min(target_low);
            } else if state.on_stack.contains(target) {
                let target_index = *state.indices.get(target).expect("target index");
                let lowlink = state.lowlinks.get_mut(node).expect("node lowlink");
                *lowlink = (*lowlink).min(target_index);
            }
        }

        if state.lowlinks.get(node) == state.indices.get(node) {
            let mut component = Vec::new();
            while let Some(entry) = state.stack.pop() {
                state.on_stack.remove(&entry);
                component.push(entry.clone());
                if entry == node {
                    break;
                }
            }
            component.sort();
            if component.len() > 1
                || (state.include_self_loops
                    && graph
                        .get(node)
                        .is_some_and(|targets| targets.contains(node)))
            {
                state.components.push(component);
            }
        }
    }

    let mut adjacency = graph.clone();
    for targets in graph.values() {
        for target in targets {
            adjacency.entry(target.clone()).or_default();
        }
    }

    let mut state = TarjanState {
        index: 0,
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        indices: BTreeMap::new(),
        lowlinks: BTreeMap::new(),
        components: Vec::new(),
        include_self_loops,
    };
    for node in adjacency.keys() {
        if !state.indices.contains_key(node) {
            strong_connect(node, &adjacency, &mut state);
        }
    }
    state.components.sort();
    state.components
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::strongly_connected_components;

    #[test]
    fn excludes_self_loops_when_disabled() {
        let mut graph = BTreeMap::new();
        graph.insert("a".to_owned(), BTreeSet::from(["a".to_owned()]));
        assert!(strongly_connected_components(&graph, false).is_empty());
    }

    #[test]
    fn includes_self_loops_when_enabled() {
        let mut graph = BTreeMap::new();
        graph.insert("a".to_owned(), BTreeSet::from(["a".to_owned()]));
        assert_eq!(
            strongly_connected_components(&graph, true),
            vec![vec!["a".to_owned()]]
        );
    }
}
