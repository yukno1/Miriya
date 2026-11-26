//! 资产依赖图：cook 闭包与热重载反向失效拓扑序（ADR-0013）。

use std::collections::{HashMap, HashSet};

use crate::AssetId;
use crate::error::{AssetError, Result};

/// 有向依赖图：边 `dependent -> dependency`。
#[derive(Debug, Default)]
pub struct DependencyGraph {
    edges: HashMap<AssetId, HashSet<AssetId>>,
}

impl DependencyGraph {
    /// 创建空图。
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加一条依赖边。
    pub fn add_edge(&mut self, dependent: AssetId, dependency: AssetId) {
        if dependent != dependency {
            self.edges.entry(dependent).or_default().insert(dependency);
        }
    }

    /// 替换某资产的直接依赖（去重、忽略自环）。
    pub fn replace_dependencies(&mut self, dependent: AssetId, dependencies: &[AssetId]) {
        self.edges.remove(&dependent);
        for dependency in dependencies {
            self.add_edge(dependent, *dependency);
        }
    }

    /// 读取直接依赖。
    pub fn dependencies_of(&self, id: AssetId) -> Vec<AssetId> {
        sorted(self.edges.get(&id).into_iter().flatten().copied())
    }

    /// 读取直接依赖者。
    pub fn dependents_of(&self, id: AssetId) -> Vec<AssetId> {
        sorted(
            self.edges
                .iter()
                .filter(|(_, deps)| deps.contains(&id))
                .map(|(dependent, _)| *dependent),
        )
    }

    /// 某个资产（含自身）的全部下游依赖者闭包。
    pub fn reverse_closure(&self, roots: &[AssetId]) -> Vec<AssetId> {
        let mut reverse: HashMap<AssetId, Vec<AssetId>> = HashMap::new();
        for (dependent, dependencies) in &self.edges {
            for dependency in dependencies {
                reverse.entry(*dependency).or_default().push(*dependent);
            }
        }
        let mut seen: HashSet<AssetId> = HashSet::new();
        let mut queue: Vec<AssetId> = roots.to_vec();
        while let Some(current) = queue.pop() {
            if !seen.insert(current) {
                continue;
            }
            if let Some(children) = reverse.get(&current) {
                queue.extend(children.iter().copied());
            }
        }
        sorted(seen.into_iter())
    }

    /// cook 顺序：依赖先于依赖者。
    pub fn cook_order(&self, roots: &[AssetId]) -> Result<Vec<AssetId>> {
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        for root in roots {
            self.visit_dependencies(*root, &mut visiting, &mut visited, &mut order)?;
        }
        Ok(order)
    }

    /// 失效顺序：依赖者先于其依赖，仅覆盖受影响的下游闭包。
    pub fn invalidation_order(&self, roots: &[AssetId]) -> Result<Vec<AssetId>> {
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        for root in roots {
            self.visit_dependents(*root, &mut visiting, &mut visited, &mut order)?;
        }
        Ok(order)
    }

    fn visit_dependents(
        &self,
        id: AssetId,
        visiting: &mut HashSet<AssetId>,
        visited: &mut HashSet<AssetId>,
        order: &mut Vec<AssetId>,
    ) -> Result<()> {
        if visited.contains(&id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(AssetError::Cycle(id));
        }
        for dependent in self.dependents_of(id) {
            self.visit_dependents(dependent, visiting, visited, order)?;
        }
        visiting.remove(&id);
        visited.insert(id);
        order.push(id);
        Ok(())
    }

    /// 检测从给定根出发的依赖环。
    pub fn detect_cycle(&self, roots: &[AssetId]) -> Result<()> {
        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        for root in roots {
            self.visit_dependencies(*root, &mut visiting, &mut visited, &mut order)?;
        }
        Ok(())
    }

    fn visit_dependencies(
        &self,
        id: AssetId,
        visiting: &mut HashSet<AssetId>,
        visited: &mut HashSet<AssetId>,
        order: &mut Vec<AssetId>,
    ) -> Result<()> {
        if visited.contains(&id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(AssetError::Cycle(id));
        }
        for dependency in self.dependencies_of(id) {
            self.visit_dependencies(dependency, visiting, visited, order)?;
        }
        visiting.remove(&id);
        visited.insert(id);
        order.push(id);
        Ok(())
    }
}

fn sorted(iter: impl Iterator<Item = AssetId>) -> Vec<AssetId> {
    let mut values: Vec<AssetId> = iter.collect();
    values.sort_unstable();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cook_and_invalidation_orders_are_reversed() {
        let mut graph = DependencyGraph::new();
        let scene = AssetId::new();
        let mesh = AssetId::new();
        let texture = AssetId::new();
        graph.add_edge(scene, mesh);
        graph.add_edge(mesh, texture);

        assert_eq!(
            graph.cook_order(&[scene]).unwrap(),
            vec![texture, mesh, scene]
        );
        assert_eq!(
            graph.invalidation_order(&[mesh]).unwrap(),
            vec![scene, mesh]
        );
        assert_eq!(
            graph.reverse_closure(&[texture]),
            vec![scene, mesh, texture]
        );
    }

    #[test]
    fn cycle_is_detected() {
        let mut graph = DependencyGraph::new();
        let a = AssetId::new();
        let b = AssetId::new();
        graph.add_edge(a, b);
        graph.add_edge(b, a);
        assert!(matches!(
            graph.detect_cycle(&[a]),
            Err(AssetError::Cycle(_))
        ));
    }
}
