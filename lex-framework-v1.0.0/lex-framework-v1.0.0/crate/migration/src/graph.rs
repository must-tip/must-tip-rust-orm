//! Deterministic dependency graph for typed migrations.

use anyhow::{Result, anyhow};
use std::collections::{BTreeMap, BTreeSet};

use crate::operation::MigrationOperation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationNode {
    pub id: String,
    pub dependencies: Vec<String>,
    pub operations: Vec<MigrationOperation>,
}

#[derive(Debug, Clone)]
pub struct MigrationGraph {
    nodes: BTreeMap<String, MigrationNode>,
}

impl MigrationGraph {
    pub fn new(nodes: impl IntoIterator<Item = MigrationNode>) -> Result<Self> {
        let mut map = BTreeMap::new();
        for node in nodes {
            validate_migration_id(&node.id)?;
            if node.operations.is_empty() {
                return Err(anyhow!("migration {} contains no operations", node.id));
            }
            if node
                .dependencies
                .iter()
                .any(|dependency| dependency == &node.id)
            {
                return Err(anyhow!("migration {} depends on itself", node.id));
            }
            if map.insert(node.id.clone(), node).is_some() {
                return Err(anyhow!("duplicate migration id"));
            }
        }
        for node in map.values() {
            for dependency in &node.dependencies {
                if !map.contains_key(dependency) {
                    return Err(anyhow!(
                        "migration {} has missing dependency {}",
                        node.id,
                        dependency
                    ));
                }
            }
        }
        let graph = Self { nodes: map };
        let _ = graph.plan()?;
        Ok(graph)
    }

    pub fn plan(&self) -> Result<Vec<&MigrationNode>> {
        let mut remaining_dependencies = self
            .nodes
            .iter()
            .map(|(id, node)| {
                (
                    id.clone(),
                    node.dependencies.iter().cloned().collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut ready = remaining_dependencies
            .iter()
            .filter_map(|(id, dependencies)| dependencies.is_empty().then_some(id.clone()))
            .collect::<BTreeSet<_>>();
        let mut plan = Vec::with_capacity(self.nodes.len());

        while let Some(id) = ready.pop_first() {
            let node = self
                .nodes
                .get(&id)
                .ok_or_else(|| anyhow!("migration graph became inconsistent"))?;
            plan.push(node);
            remaining_dependencies.remove(&id);
            for (candidate, dependencies) in &mut remaining_dependencies {
                dependencies.remove(&id);
                if dependencies.is_empty() {
                    ready.insert(candidate.clone());
                }
            }
        }

        if !remaining_dependencies.is_empty() {
            return Err(anyhow!(
                "migration dependency cycle detected among: {}",
                remaining_dependencies
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(plan)
    }

    pub fn render_up(&self) -> Result<Vec<(String, Vec<String>)>> {
        self.plan()?
            .into_iter()
            .map(|node| {
                let mut statements = Vec::new();
                for operation in &node.operations {
                    statements.extend(operation.up_sql()?);
                }
                Ok((node.id.clone(), statements))
            })
            .collect()
    }

    pub fn render_down(&self) -> Result<Vec<(String, Vec<String>)>> {
        let mut plan = self.plan()?;
        plan.reverse();
        plan.into_iter()
            .map(|node| {
                let mut statements = Vec::new();
                for operation in node.operations.iter().rev() {
                    statements.extend(operation.down_sql()?);
                }
                Ok((node.id.clone(), statements))
            })
            .collect()
    }
}

fn validate_migration_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Err(anyhow!("invalid migration id: {value}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_controller::ColumnSpec;

    fn op(name: &str) -> MigrationOperation {
        MigrationOperation::AddColumn {
            table: "users".into(),
            column: name.into(),
            spec: ColumnSpec {
                sql_type: "TEXT".into(),
                nullable: true,
                unique: false,
                primary_key: false,
                default: None,
                encrypted: false,
            },
        }
    }

    #[test]
    fn graph_orders_dependencies_and_reverse_plan() {
        let graph = MigrationGraph::new([
            MigrationNode {
                id: "0002_b".into(),
                dependencies: vec!["0001_a".into()],
                operations: vec![op("b")],
            },
            MigrationNode {
                id: "0001_a".into(),
                dependencies: vec![],
                operations: vec![op("a")],
            },
        ])
        .unwrap();
        assert_eq!(
            graph
                .plan()
                .unwrap()
                .iter()
                .map(|node| node.id.as_str())
                .collect::<Vec<_>>(),
            vec!["0001_a", "0002_b"]
        );
        assert_eq!(
            graph
                .render_down()
                .unwrap()
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            vec!["0002_b", "0001_a"]
        );
    }

    #[test]
    fn graph_rejects_missing_dependencies_and_cycles() {
        assert!(
            MigrationGraph::new([MigrationNode {
                id: "0001_a".into(),
                dependencies: vec!["missing".into()],
                operations: vec![op("a")]
            }])
            .is_err()
        );
        assert!(
            MigrationGraph::new([
                MigrationNode {
                    id: "a".into(),
                    dependencies: vec!["b".into()],
                    operations: vec![op("a")]
                },
                MigrationNode {
                    id: "b".into(),
                    dependencies: vec!["a".into()],
                    operations: vec![op("b")]
                },
            ])
            .is_err()
        );
    }
}
