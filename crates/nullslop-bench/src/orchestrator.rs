//! Bench execution plan — cartesian product of tasks × models.
//!
//! Provides [`BenchPlan`] which holds the ordered list of (task, model) pairs
//! and a lookup from task name to task definition. Built from CLI arguments
//! via [`build_plan`].

use std::collections::HashMap;

use globset::GlobBuilder;
use error_stack::ResultExt;

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

/// Returns the sorted list of all available task names.
///
/// Useful for help text: `Available tasks: edit-*, fix-*, redirect-*, hello-world, …`.
pub fn list_task_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = tasks::bench_tasks().iter().map(|t| t.name).collect();
    names.sort_unstable();
    names
}

/// Builds a [`BenchPlan`] from CLI arguments.
///
/// Each element of `task_patterns` is treated as a glob pattern (e.g. `"edit-*"`,
/// `"*-broken-*"`, `"hello-world"`). A bare name with no glob characters matches
/// exactly one task. If `task_patterns` is empty, all tasks are included.
/// Computes the cartesian product of tasks × models (one pair per task per model).
///
/// # Errors
///
/// Returns an error if any glob pattern is invalid.
pub fn build_plan(
    models: &[String],
    task_patterns: &[String],
) -> Result<BenchPlan, error_stack::Report<globset::Error>> {
    let all_tasks = tasks::bench_tasks();
    let filtered: Vec<&BenchTask> = if task_patterns.is_empty() {
        all_tasks.iter().collect()
    } else {
        let matchers = compile_patterns(task_patterns)?;
        all_tasks
            .iter()
            .filter(|t| matchers.iter().any(|m| m.is_match(t.name)))
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

    Ok(BenchPlan { pairs, task_lookup })
}

/// Compiles glob patterns into matchers.
fn compile_patterns(
    patterns: &[String],
) -> Result<Vec<globset::GlobMatcher>, error_stack::Report<globset::Error>> {
    patterns
        .iter()
        .map(|p| {
            GlobBuilder::new(p)
                .literal_separator(true)
                .build()
                .attach(p.clone())
                .map(|g| g.compile_matcher())
        })
        .collect()
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
        let plan = build_plan(&models, &task_names).expect("plan");

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
        let plan = build_plan(&models, &task_names).expect("plan");

        // Then all tasks are included (one pair per task).
        assert!(plan.pairs.len() > 1);
    }

    #[test]
    fn build_plan_filters_to_specified_tasks() {
        // Given a task filter with one task.
        let models = vec!["model-a".to_owned()];
        let task_names = vec!["hello-world".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names).expect("plan");

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
        let plan = build_plan(&models, &task_names).expect("plan");

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
        let plan = build_plan(&models, &task_names).expect("plan");

        // Then no pairs are produced.
        assert!(plan.pairs.is_empty());
    }

    #[test]
    fn build_plan_multiple_models_multiple_tasks() {
        // Given 2 models and 2 tasks.
        let models = vec!["model-a".to_owned(), "model-b".to_owned()];
        let task_names = vec!["hello-world".to_owned(), "json-parser".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names).expect("plan");

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

    #[test]
    fn build_plan_glob_matches_prefix() {
        // Given a glob filter for all edit tasks.
        let models = vec!["model-a".to_owned()];
        let task_names = vec!["edit-*".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names).expect("plan");

        // Then only edit tasks appear.
        assert!(plan.pairs.len() > 5);
        assert!(plan.pairs.iter().all(|(name, _)| name.starts_with("edit-")));
    }

    #[test]
    fn build_plan_glob_matches_infix() {
        // Given a glob filter for broken tasks.
        let models = vec!["model-a".to_owned()];
        let task_names = vec!["*-broken-*".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names).expect("plan");

        // Then broken tasks appear (fix-syntax-broken-rust, fix-syntax-broken-python).
        assert_eq!(plan.pairs.len(), 2);
        assert!(plan
            .pairs
            .iter()
            .all(|(name, _)| name.contains("-broken-")));
    }

    #[test]
    fn build_plan_multiple_globs_union() {
        // Given two glob patterns.
        let models = vec!["model-a".to_owned()];
        let task_names = vec!["hello-world".to_owned(), "edit-*".to_owned()];

        // When building the plan.
        let plan = build_plan(&models, &task_names).expect("plan");

        // Then all edit tasks + hello-world appear.
        let names: Vec<&str> = plan.pairs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"hello-world"));
        assert!(names.iter().all(|n| n.starts_with("edit-") || *n == "hello-world"));
    }

    #[test]
    fn list_task_names_returns_sorted_names() {
        let names = list_task_names();
        assert!(!names.is_empty());
        assert_eq!(names, {
            let mut sorted = names.clone();
            sorted.sort_unstable();
            sorted
        });
    }
}
