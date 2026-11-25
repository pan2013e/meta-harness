use std::collections::{HashMap, HashSet};

/// In-memory Wait-For Graph tracking active synchronous `peer_consult` edges (`caller -> target`)
/// to detect and reject deadlocks in O(V + E).
#[derive(Debug, Default)]
pub struct WaitForGraph {
    edges: HashMap<String, Vec<String>>,
}

impl WaitForGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attempts to add directed consult edge `caller -> target`.
    /// Returns `Err(cycle_path)` if `caller == target` or if `target` can already
    /// transitively reach `caller`.
    pub fn try_acquire_edge(&mut self, caller: &str, target: &str) -> Result<(), Vec<String>> {
        if caller == target {
            return Err(vec![caller.to_string(), target.to_string()]);
        }

        if let Some(mut path_to_caller) = self.find_path(target, caller) {
            let mut cycle = Vec::with_capacity(path_to_caller.len() + 1);
            cycle.push(caller.to_string());
            cycle.append(&mut path_to_caller);
            return Err(cycle);
        }

        self.edges
            .entry(caller.to_string())
            .or_default()
            .push(target.to_string());
        Ok(())
    }

    /// Releases one active directed edge `caller -> target`.
    pub fn release_edge(&mut self, caller: &str, target: &str) {
        if let Some(targets) = self.edges.get_mut(caller) {
            if let Some(idx) = targets.iter().position(|t| t == target) {
                targets.swap_remove(idx);
            }
            if targets.is_empty() {
                self.edges.remove(caller);
            }
        }
    }

    /// Removes all edges associated with a cancelled/stopped member.
    pub fn clear_member(&mut self, member: &str) {
        self.edges.remove(member);
        self.edges.retain(|_, targets| {
            targets.retain(|t| t != member);
            !targets.is_empty()
        });
    }

    pub fn clear(&mut self) {
        self.edges.clear();
    }

    fn find_path(&self, start: &str, goal: &str) -> Option<Vec<String>> {
        let mut visited = HashSet::new();
        let mut stack = vec![start.to_string()];
        self.dfs(start, goal, &mut visited, &mut stack)
            .then_some(stack)
    }

    fn dfs(
        &self,
        node: &str,
        goal: &str,
        visited: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> bool {
        if node == goal {
            return true;
        }
        if !visited.insert(node.to_string()) {
            return false;
        }
        if let Some(neighbors) = self.edges.get(node) {
            for next in neighbors {
                path.push(next.clone());
                if self.dfs(next, goal, visited, path) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_direct_and_transitive_cycles() {
        let mut g = WaitForGraph::new();
        assert!(g.try_acquire_edge("claude", "codex").is_ok());
        assert!(g.try_acquire_edge("codex", "antigravity").is_ok());

        // Self cycle
        assert!(g.try_acquire_edge("claude", "claude").is_err());

        // Direct cycle codex -> claude
        let direct = g.try_acquire_edge("codex", "claude").unwrap_err();
        assert_eq!(direct, vec!["codex", "claude", "codex"]);

        // Transitive 3-node cycle antigravity -> claude
        let transitive = g.try_acquire_edge("antigravity", "claude").unwrap_err();
        assert_eq!(
            transitive,
            vec!["antigravity", "claude", "codex", "antigravity"]
        );

        // Once released, edge succeeds
        g.release_edge("codex", "antigravity");
        assert!(g.try_acquire_edge("antigravity", "claude").is_ok());
    }
}
