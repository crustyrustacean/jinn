//! Bench execution plan — cartesian product of tasks × models.
//!
//! Provides [`BenchPlan`] which holds the ordered list of (task, model) pairs
//! and a lookup from task name to task definition. Built from CLI arguments
//! via [`build_plan`].

use std::collections::HashMap;

use crate::task::BenchTask;
use crate::tasks;

/// The full execution plan for a bench run.
///
/// Contains an ordered list of (task_name, model) pairs and a lookup
/// from task name to the task definition (for messages and verification).
#[derive(Debug)]
pub struct BenchPlan {
    /// Ordered list of (task_name, model) pairs to execute sequentially.
    pub pairs: Vec<(String, String)>,
    /// Map from task name to task definition.
    pub task_lookup: HashMap<String, BenchTask>,
}

/// Builds a [`BenchPlan`] from CLI arguments.
///
/// Filters tasks if `task_names` is non-empty; otherwise includes all tasks.
/// Computes the cartesian product of models × tasks (one pair per task per model).
pub fn build_plan(models: &[String], task_names: &[String]) -> BenchPlan {
    let all_tasks = tasks::bench_tasks();
    let filtered: Vec<&BenchTask> = if task_names.is_empty() {
        all_tasks.iter().collect()
    } else {
        all_tasks
            .iter()
            .filter(|t| task_names.contains(&t.name.to_owned()))
            .collect()
    };

    let task_lookup: HashMap<String, BenchTask> = filtered
        .iter()
        .map(|t| (t.name.to_owned(), (*t).clone()))
        .collect();

    // Cartesian product: for each task, for each model.
    let mut pairs = Vec::new();
    for task in &filtered {
        for model in models {
            pairs.push((task.name.to_owned(), model.clone()));
        }
    }

    BenchPlan { pairs, task_lookup }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, reason = "test code")]

    use super::*;

    #[test]
    fn build_plan_creates_cartesian_product() {
        // Given 2 models and 1 task.
        let models = vec!["model-a".to_owned(), "model-b".to_owned()];
        let task_names = vec!["hello-world".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names);

        // Then the plan has 2 pairs.
        assert_eq!(plan.pairs.len(), 2);
        assert_eq!(
            plan.pairs[0],
            ("hello-world".to_owned(), "model-a".to_owned())
        );
        assert_eq!(
            plan.pairs[1],
            ("hello-world".to_owned(), "model-b".to_owned())
        );
    }

    #[test]
    fn build_plan_includes_all_tasks_when_filter_empty() {
        // Given empty task filter.
        let models = vec!["model-a".to_owned()];
        let task_names: Vec<String> = vec![];

        // When building the plan.
        let plan = build_plan(&models, &task_names);

        // Then all tasks are included (one pair per task).
        assert!(plan.pairs.len() > 1);
    }

    #[test]
    fn build_plan_filters_to_specified_tasks() {
        // Given a task filter with one task.
        let models = vec!["model-a".to_owned()];
        let task_names = vec!["hello-world".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names);

        // Then only that task appears.
        assert_eq!(plan.pairs.len(), 1);
        assert_eq!(plan.pairs[0].0, "hello-world");
    }

    #[test]
    fn build_plan_task_lookup_contains_definitions() {
        // Given a plan with hello-world.
        let models = vec!["model-a".to_owned()];
        let task_names = vec!["hello-world".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names);

        // Then the task lookup has the definition.
        assert!(plan.task_lookup.contains_key("hello-world"));
        let task = plan.task_lookup.get("hello-world").expect("task");
        assert!(!task.messages.is_empty());
    }

    #[test]
    fn build_plan_with_unknown_task_name_produces_no_pairs() {
        // Given a task filter with a nonexistent task.
        let models = vec!["model-a".to_owned()];
        let task_names = vec!["nonexistent-task".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names);

        // Then no pairs are produced.
        assert!(plan.pairs.is_empty());
    }

    #[test]
    fn build_plan_multiple_models_multiple_tasks() {
        // Given 2 models and 2 tasks.
        let models = vec!["model-a".to_owned(), "model-b".to_owned()];
        let task_names = vec!["hello-world".to_owned(), "json-parser".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names);

        // Then the plan has 4 pairs (2 models × 2 tasks).
        assert_eq!(plan.pairs.len(), 4);
        // Pairs are ordered: tasks × models.
        assert_eq!(plan.pairs[0].0, "hello-world");
        assert_eq!(plan.pairs[0].1, "model-a");
        assert_eq!(plan.pairs[1].1, "model-b");
        assert_eq!(plan.pairs[2].0, "json-parser");
        assert_eq!(plan.pairs[2].1, "model-a");
        assert_eq!(plan.pairs[3].1, "model-b");
    }
}
