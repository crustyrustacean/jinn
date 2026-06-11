#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::sync::Arc;

use kameo::prelude::Spawn;

use crate::common::actor_deps::ActorDeps;
use crate::common::app_paths::AppPaths;
use crate::common::app_state::AppState;
use crate::common::services::bus_service::BusService;
use crate::common::services::test_services::TestServices;
use crate::common::state::State;
use crate::common::bus::test_harness::{Recorder, TestHarness, await_recorded};
use jinn_selection_widget::PreviewCache;
use crate::feat::session_lifecycle::protocol::event::{
    SessionCreated, SessionCwdChanged, SessionSetupCompleted,
};
use crate::init::env_init_actor::EnvironmentLoaded;

use super::*;

/// Creates a temp dir containing a single skill `test-skill`.
fn skill_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    let skills_base = dir.path().join(".agents/skills/test-skill");
    std::fs::create_dir_all(&skills_base).expect("create skill dir");
    std::fs::write(
        skills_base.join("SKILL.md"),
        "---\nname: test-skill\ndescription: A test skill\n---\n\n# Content",
    )
    .expect("write SKILL.md");
    dir
}

/// Writes a skill `name` into a directory's `skills/<name>/SKILL.md`.
async fn write_skill(base: &std::path::Path, name: &str, body: &str) {
    let dir = base.join("skills").join(name);
    std::fs::create_dir_all(&dir).expect("create skill dir");
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {name} skill\n---\n\n{body}"),
    )
    .expect("write SKILL.md");
}

async fn create_harness_with_paths(paths: AppPaths) -> (TestHarness, State, ActorDeps) {
    let harness = TestHarness::new().await;
    let state = State::new(AppState::default());
    let mut services = TestServices::builder().paths(paths).build();
    services.bus = harness.bus().clone();
    let deps = ActorDeps {
        services: services.clone(),
    };
    (harness, state, deps)
}

async fn spawn_actor(deps: &ActorDeps, state: &State) -> ActorRef<SkillsScanActor> {
    let actor = SkillsScanActor::spawn(SkillsScanActorDeps {
        deps: deps.clone(),
        state: state.clone(),
    });
    actor.wait_for_startup().await;
    actor
}

#[tokio::test]
async fn scan_skills_command_writes_to_app_state() {
    // Given an actor with a temp directory containing a skill.
    let dir = tempfile::tempdir().expect("create temp dir");
    let skills_base = dir.path().join(".agents/skills/test-skill");
    std::fs::create_dir_all(&skills_base).expect("create skill dir");
    std::fs::write(
        skills_base.join("SKILL.md"),
        "---\nname: test-skill\ndescription: A test skill\n---\n\n# Content",
    )
    .expect("write SKILL.md");

    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    {
        let mut guard = state.write();
        guard
            .session
            .active_session_mut()
            .set_cwd(dir.path().to_path_buf());
    }
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;

    // When publishing ScanSkills command.
    harness.publish(ScanSkills {
        session_id: session_id.clone(),
    }).await;

    // Then skills are written to the session's ephemeral discovered set.
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session exists");
    assert_eq!(session.discovered_skills().len(), 1);
    assert_eq!(session.discovered_skills()[0].name, "test-skill");
}

#[tokio::test]
async fn session_created_event_scans_skills() {
    // Given an actor whose active session cwd contains a skill.
    let dir = skill_dir();
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    {
        let mut guard = state.write();
        guard
            .session
            .active_session_mut()
            .set_cwd(dir.path().to_path_buf());
    }
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;

    // When publishing SessionCreated for that session.
    harness.publish(SessionCreated {
        session_id: session_id.clone(),
    }).await;

    // Then the skill is written to the session's discovered set.
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session exists");
    assert_eq!(session.discovered_skills().len(), 1);
}

#[tokio::test]
async fn session_created_event_skips_scan_when_cwd_is_sentinel() {
    // Given an actor whose active session cwd is the pending "." sentinel.
    let dir = skill_dir();
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    // Note: deliberately do NOT set a real cwd; default is ".".
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;

    // When publishing SessionCreated for the sentinel-cwd session.
    harness.publish(SessionCreated {
        session_id: session_id.clone(),
    }).await;

    // Then no scan runs: the discovered set stays empty.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session exists");
    assert!(session.discovered_skills().is_empty());
}

#[tokio::test]
async fn session_setup_completed_event_scans_skills() {
    // Given an actor whose active session cwd contains a skill.
    let dir = skill_dir();
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    {
        let mut guard = state.write();
        guard
            .session
            .active_session_mut()
            .set_cwd(dir.path().to_path_buf());
    }
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;

    // When publishing SessionSetupCompleted.
    harness.publish(SessionSetupCompleted {
        session_id: session_id.clone(),
        cwd: dir.path().to_path_buf(),
        error: None,
    }).await;

    // Then the skill is written to the session's discovered set.
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session exists");
    assert_eq!(session.discovered_skills().len(), 1);
}

#[tokio::test]
async fn session_cwd_changed_event_scans_skills() {
    // Given an actor whose active session cwd contains a skill.
    let dir = skill_dir();
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    {
        let mut guard = state.write();
        guard
            .session
            .active_session_mut()
            .set_cwd(dir.path().to_path_buf());
    }
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;

    // When publishing SessionCwdChanged.
    harness.publish(SessionCwdChanged {
        session_id: session_id.clone(),
        cwd: dir.path().to_path_buf(),
    }).await;

    // Then the skill is written to the session's discovered set.
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session exists");
    assert_eq!(session.discovered_skills().len(), 1);
}

#[tokio::test]
async fn environment_loaded_event_scans_active_session_skills() {
    // Given an actor whose active session cwd contains a skill.
    let dir = skill_dir();
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    {
        let mut guard = state.write();
        guard
            .session
            .active_session_mut()
            .set_cwd(dir.path().to_path_buf());
    }
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;

    // When publishing EnvironmentLoaded.
    harness.publish(EnvironmentLoaded {
        config: crate::ProvidersConfig {
            providers: vec![],
            aliases: vec![],
            default_provider: None,
        },
    }).await;

    // Then the active session's skill is discovered.
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session exists");
    assert_eq!(session.discovered_skills().len(), 1);
}

#[tokio::test]
async fn scan_skills_clears_skill_preview_cache() {
    // Given an actor whose state holds a populated preview cache (from a
    // previous picker session).
    let dir = tempfile::tempdir().expect("create temp dir");
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    {
        let guard = state.write();
        guard.frontend.caches.skill_preview_cache.write().insert(
            "stale-skill".to_owned(),
            80,
            vec![ratatui::text::Line::raw("stale")],
        );
    }
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;

    // When publishing ScanSkills command (rescan).
    harness.publish(ScanSkills {
        session_id,
    }).await;

    // Then the cache is cleared so rescanned bodies are re-rendered fresh.
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    assert!(
        state
            .read()
            .frontend
            .caches
            .skill_preview_cache
            .read()
            .is_empty(),
        "rescan must clear the skill preview cache"
    );
}

#[tokio::test]
async fn scan_skills_command_emits_skills_loaded() {
    // Given an actor with a temp directory.
    let dir = tempfile::tempdir().expect("create temp dir");
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;

    // When publishing ScanSkills command.
    harness.publish(ScanSkills {
        session_id,
    }).await;

    // Then SkillsLoaded event is emitted.
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());
    let loaded = &recorded[0];
    assert!(loaded.error.is_none());
}

#[tokio::test]
async fn scan_skills_empty_dir_emits_empty_loaded() {
    // Given an actor with an empty temp directory.
    let dir = tempfile::tempdir().expect("create temp dir");
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;

    // When publishing ScanSkills command.
    harness.publish(ScanSkills {
        session_id,
    }).await;

    // Then SkillsLoaded has empty skills list.
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());
    let loaded = &recorded[0];
    assert!(loaded.skills.is_empty());
    assert!(loaded.error.is_none());
}

#[tokio::test]
async fn scan_skills_replacing_cwd_clears_previous_discovered_skills() {
    // Given a session whose cwd has a project skill, scanned once so the
    // discovered set contains it.
    let home = tempfile::tempdir().expect("create home dir");
    let dir_with_skill = home.path().join("populated");
    std::fs::create_dir_all(&dir_with_skill).expect("create populated dir");
    let skill_dir_path = dir_with_skill.join(".agents").join("skills").join("alpha");
    std::fs::create_dir_all(&skill_dir_path).expect("create project skill dir");
    std::fs::write(
        skill_dir_path.join("SKILL.md"),
        "---\nname: alpha\ndescription: alpha skill\n---\n\n# A",
    )
    .expect("write project SKILL.md");

    let empty_dir = home.path().join("empty");
    std::fs::create_dir_all(&empty_dir).expect("create empty dir");

    let paths_root = tempfile::tempdir().expect("create temp dir for AppPaths");
    let state = State::new(AppState::default());
    let session_id = state.read().session.active_session_id().clone();
    {
        let mut guard = state.write();
        guard
            .session
            .active_session_mut()
            .set_cwd(dir_with_skill.clone());
    }

    let harness = TestHarness::new().await;
    let mut paths = AppPaths::new_in(paths_root.path());
    paths.set_home_dir_for_test(home.path().to_path_buf());
    let mut services = TestServices::builder().paths(paths).build();
    services.bus = harness.bus().clone();
    let deps = ActorDeps {
        services: services.clone(),
    };
    let _actor = spawn_actor(&deps, &state).await;
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;

    // First scan: discovers `alpha`.
    harness.publish(ScanSkills {
        session_id: session_id.clone(),
    }).await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    {
        let guard = state.read();
        let session = guard
            .session
            .get(&session_id)
            .expect("session exists after first scan");
        let skills = session.discovered_skills();
        assert_eq!(skills.len(), 1, "populated cwd yields one skill");
        assert_eq!(skills[0].name, "alpha");
    }

    // When the cwd changes to an empty dir and a second scan runs.
    {
        let mut guard = state.write();
        guard
            .session
            .active_session_mut()
            .set_cwd(empty_dir.clone());
    }
    harness.publish(ScanSkills {
        session_id: session_id.clone(),
    }).await;
    let recorded2 = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded2.is_empty(), "second scan should emit SkillsLoaded");

    // Then the discovered set is empty — no stale `alpha` carryover.
    let guard = state.read();
    let session = guard
        .session
        .get(&session_id)
        .expect("session exists after second scan");
    assert!(
        session.discovered_skills().is_empty(),
        "empty cwd must clear previously discovered skills"
    );
}

#[tokio::test]
async fn scan_skills_nonexistent_dir_emits_empty_loaded() {
    // Given an actor with a nonexistent directory.
    let dir = tempfile::tempdir().expect("create temp dir");
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;

    // When publishing ScanSkills command.
    harness.publish(ScanSkills {
        session_id,
    }).await;

    // Then SkillsLoaded has empty skills list.
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());
    let loaded = &recorded[0];
    assert!(loaded.skills.is_empty());
    assert!(loaded.error.is_none());
}

#[tokio::test]
async fn scan_skills_project_overrides_global_same_name() {
    // Given a global skill `shared` and a project skill `shared` at the cwd.
    let dir = tempfile::tempdir().expect("create temp dir");
    // Global skills live at dir/skills (AppPaths::new_in(dir)).
    write_skill(dir.path(), "shared", "# GLOBAL body");
    // Project skill lives at dir/.agents/skills/shared.
    let project_skill_dir = dir.path().join(".agents/skills/shared");
    std::fs::create_dir_all(&project_skill_dir).expect("create project skill dir");
    std::fs::write(
        project_skill_dir.join("SKILL.md"),
        "---\nname: shared\ndescription: shared skill\n---\n\n# PROJECT body",
    )
    .expect("write project SKILL.md");

    // And home is set ABOVE dir so the walk reaches the project layer.
    let (harness, state, deps) = create_harness_with_paths(AppPaths::new_in(dir.path())).await;
    let session_id = state.read().session.active_session_id().clone();
    let _actor = spawn_actor(&deps, &state).await;

    // When scanning.
    harness.publish(ScanSkills {
        session_id: session_id.clone(),
    }).await;

    // Then exactly one `shared` skill exists and it is the PROJECT one.
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session exists");
    let skills = session.discovered_skills();
    assert_eq!(skills.len(), 1, "dedup to one `shared`");
    assert_eq!(skills[0].name, "shared");
    assert!(
        skills[0].body.contains("PROJECT body"),
        "project wins: {body}",
        body = skills[0].body
    );
}

#[tokio::test]
async fn scan_skills_discovers_ancestor_project_skill_from_nested_cwd() {
    // Given a tree home/repo/.agents/skills/ancestor and a session whose
    // cwd is home/repo/subdir (a descendant), with home set to home.
    // No VCS marker anywhere, so the walk is bounded by exclusive $HOME.
    let home = tempfile::tempdir().expect("create home dir");
    let repo = home.path().join("repo");
    let subdir = repo.join("subdir");
    std::fs::create_dir_all(&subdir).expect("create nested dirs");
    let ancestor_skill = repo.join(".agents/skills/ancestor/SKILL.md");
    std::fs::create_dir_all(ancestor_skill.parent().unwrap())
        .expect("create ancestor skill dir");
    std::fs::write(
        &ancestor_skill,
        "---\nname: ancestor\ndescription: ancestor skill\n---\n\n# ancestor body",
    )
    .expect("write ancestor skill");

    let dir = tempfile::tempdir().expect("create temp dir for AppPaths");
    let state = State::new(AppState::default());
    {
        let mut guard = state.write();
        guard.session.active_session_mut().set_cwd(subdir.clone());
    }
    let session_id = state.read().session.active_session_id().clone();

    let harness = TestHarness::new().await;
    let mut paths = AppPaths::new_in(dir.path());
    paths.set_home_dir_for_test(home.path().to_path_buf());
    let mut services = TestServices::builder().paths(paths).build();
    services.bus = harness.bus().clone();
    let deps = ActorDeps {
        services: services.clone(),
    };
    let _actor = spawn_actor(&deps, &state).await;

    // When scanning from the nested cwd.
    harness.publish(ScanSkills {
        session_id: session_id.clone(),
    }).await;

    // Then the ancestor skill (one level up from cwd, within the bounded
    // walk) is discovered.
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;
    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(!recorded.is_empty());

    let guard = state.read();
    let session = guard.session.get(&session_id).expect("session exists");
    let skills = session.discovered_skills();
    assert_eq!(
        skills.len(),
        1,
        "expected only the ancestor skill, got {len}",
        len = skills.len()
    );
    assert_eq!(skills[0].name, "ancestor");
}

#[tokio::test]
async fn scan_skills_routes_discovery_per_session_cwd() {
    // Two sessions with two different cwds, each with a distinct project skill.
    // Scanning each by its own session_id must populate that session only.
    let home = tempfile::tempdir().expect("create home dir");
    let dir_a = home.path().join("a");
    let dir_b = home.path().join("b");
    std::fs::create_dir_all(&dir_a).expect("create dir a");
    std::fs::create_dir_all(&dir_b).expect("create dir b");
    // dir_a has skill `alpha`, dir_b has skill `beta`.
    for (base, name, body) in [("a", "alpha", "# A"), ("b", "beta", "# B")] {
        let skill_dir = home
            .path()
            .join(base)
            .join(".agents")
            .join("skills")
            .join(name);
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} skill\n---\n\n{body}"),
        )
        .expect("write SKILL.md");
    }

    let paths_root = tempfile::tempdir().expect("create temp dir for AppPaths");
    let state = State::new(AppState::default());
    // Session A: cwd = dir_a.
    let session_a = state.read().session.active_session_id().clone();
    {
        let mut guard = state.write();
        guard.session.active_session_mut().set_cwd(dir_a.clone());
    }
    // Session B: create + set cwd = dir_b.
    let session_b = crate::SessionId::new();
    {
        let mut guard = state.write();
        let s = guard.session.get_or_create(&session_b);
        s.set_cwd(dir_b.clone());
    }

    let harness = TestHarness::new().await;
    let mut paths = AppPaths::new_in(paths_root.path());
    paths.set_home_dir_for_test(home.path().to_path_buf());
    let mut services = TestServices::builder().paths(paths).build();
    services.bus = harness.bus().clone();
    let deps = ActorDeps {
        services: services.clone(),
    };
    let _actor = spawn_actor(&deps, &state).await;
    let recorder = harness.spawn_recorder::<SkillsLoaded>().await;

    // Scan session A, then session B.
    for id in [&session_a, &session_b] {
        harness.publish(ScanSkills {
            session_id: id.clone(),
        }).await;
    }

    let recorded = await_recorded(&recorder, 1, std::time::Duration::from_secs(2)).await;
    assert!(recorded.len() >= 2, "should have two SkillsLoaded events");

    // Then each session sees only its own skill.
    let guard = state.read();
    let skills_a = guard
        .session
        .get(&session_a)
        .expect("session a")
        .discovered_skills();
    let skills_b = guard
        .session
        .get(&session_b)
        .expect("session b")
        .discovered_skills();
    assert_eq!(skills_a.len(), 1, "session A should see only alpha");
    assert_eq!(skills_a[0].name, "alpha");
    assert_eq!(skills_b.len(), 1, "session B should see only beta");
    assert_eq!(skills_b[0].name, "beta");
}
