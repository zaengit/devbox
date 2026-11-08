use crate::Workspace;
use anyhow::{Result, anyhow};
use std::collections::{HashMap, HashSet};

/// Return prerequisites before dependants. Rejects stale references and cycles.
pub fn startup_order(workspace: &Workspace) -> Result<Vec<String>> {
    let members = workspace
        .project_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    if members.len() != workspace.project_ids.len() {
        return Err(anyhow!("workspace contains duplicate project membership"));
    }
    let mut indegree = HashMap::<String, usize>::new();
    let mut dependants = HashMap::<String, Vec<String>>::new();
    for project in &workspace.project_ids {
        indegree.insert(project.clone(), 0);
    }
    for (project, prerequisites) in &workspace.dependencies {
        if !members.contains(project) {
            return Err(anyhow!(
                "workspace dependency references a project that is not a member"
            ));
        }
        for prerequisite in prerequisites {
            if !members.contains(prerequisite) {
                return Err(anyhow!("workspace prerequisite is not a member"));
            }
            if project == prerequisite {
                return Err(anyhow!("a project cannot depend on itself"));
            }
            *indegree.get_mut(project).expect("member was inserted") += 1;
            dependants
                .entry(prerequisite.clone())
                .or_default()
                .push(project.clone());
        }
    }
    let mut ready = workspace
        .project_ids
        .iter()
        .filter(|id| indegree[*id] == 0)
        .cloned()
        .collect::<Vec<_>>();
    let mut ordered = Vec::with_capacity(members.len());
    while let Some(project) = ready.pop() {
        ordered.push(project.clone());
        for dependant in dependants.get(&project).into_iter().flatten() {
            let count = indegree.get_mut(dependant).expect("dependant was inserted");
            *count -= 1;
            if *count == 0 {
                ready.push(dependant.clone());
            }
        }
    }
    if ordered.len() != members.len() {
        return Err(anyhow!("workspace project dependencies contain a cycle"));
    }
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Workspace;

    fn workspace(dependencies: HashMap<String, Vec<String>>) -> Workspace {
        Workspace {
            id: "w".into(),
            name: "Workspace".into(),
            created_at: 0,
            project_ids: vec!["api".into(), "web".into(), "worker".into()],
            dependencies,
            service_ids: Vec::new(),
        }
    }

    #[test]
    fn orders_prerequisites_before_dependants() {
        let mut dependencies = HashMap::new();
        dependencies.insert("web".into(), vec!["api".into()]);
        dependencies.insert("worker".into(), vec!["api".into()]);
        let order = startup_order(&workspace(dependencies)).unwrap();
        assert!(
            order.iter().position(|id| id == "api").unwrap()
                < order.iter().position(|id| id == "web").unwrap()
        );
        assert!(
            order.iter().position(|id| id == "api").unwrap()
                < order.iter().position(|id| id == "worker").unwrap()
        );
    }

    #[test]
    fn rejects_cycles_and_non_members() {
        let mut cycle = HashMap::new();
        cycle.insert("api".into(), vec!["web".into()]);
        cycle.insert("web".into(), vec!["api".into()]);
        assert!(
            startup_order(&workspace(cycle))
                .unwrap_err()
                .to_string()
                .contains("cycle")
        );
        let mut missing = HashMap::new();
        missing.insert("api".into(), vec!["outside".into()]);
        assert!(startup_order(&workspace(missing)).is_err());
    }
}
