//! Scratch-repo integration tests for the autocoder lifecycle scripts.
//!
//! The shipped `autocoder-fossil-root` and `autocoder-fossil-stage`
//! commands are read from the shipped `res/workflows/autocoder/lifecycle.toml`
//! — parsed through the production lifecycle schema, never copied into test
//! constants — rendered through the real [`CommandTemplate`] path with
//! tool-supplied arguments, then executed against a real fossil
//! repository in a temp directory. They assert the run's trunk
//! invariant at the level that matters — the repository itself:
//!
//! - the root script forks the run branch off trunk and never advances
//!   trunk;
//! - the stage script forks off the named base branch (the run branch,
//!   not trunk);
//! - the stage teardown closes the checkout without merging or
//!   deleting anything anywhere — after every stage's full lifecycle,
//!   trunk's leaf is exactly the check-in it started as;
//! - the stage's own finishing merge (the workflow's step, replayed
//!   here) lands on the run branch and still does not touch trunk;
//! - the G2 merge is the run's single trunk advance.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use jinn_preferences_config::schemas::session_lifecycle::LifecycleCommand;
use jinn_session_lifecycle_msg::CommandTemplate;
use jinn_session_lifecycle_msg::command_template::Param;

/// Reads the shipped `lifecycle.toml` and returns the named script's
/// setup command, parsed through the production schema — the same read
/// `run_lifecycle_script` resolves entries through, so the tests can
/// never drift from the bytes that ship.
fn shipped_setup(name: &str) -> String {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("res/workflows/autocoder/lifecycle.toml"),
    )
    .expect("shipped lifecycle.toml reads");
    #[derive(serde::Deserialize)]
    struct Wrapper {
        session_lifecycle: Section,
    }
    #[derive(serde::Deserialize)]
    struct Section {
        script: Vec<jinn_preferences_config::schemas::session_lifecycle::SessionLifecycle>,
    }
    let wrapper: Wrapper = toml::from_str(&text).expect("shipped lifecycle.toml deserializes");
    let script = wrapper
        .session_lifecycle
        .script
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("shipped lifecycle.toml carries {name}"));
    match script.setup.expect("setup exists") {
        LifecycleCommand::Shell(command) => command,
        LifecycleCommand::Builtin(id) => panic!("{name} ships a shell command, not builtin {id}"),
    }
}

/// Reads the shipped `lifecycle.toml` and returns the named script's
/// teardown command, through the same production-schema parse as
/// [`shipped_setup`].
fn shipped_teardown(name: &str) -> String {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("res/workflows/autocoder/lifecycle.toml"),
    )
    .expect("shipped lifecycle.toml reads");
    #[derive(serde::Deserialize)]
    struct Wrapper {
        session_lifecycle: Section,
    }
    #[derive(serde::Deserialize)]
    struct Section {
        script: Vec<jinn_preferences_config::schemas::session_lifecycle::SessionLifecycle>,
    }
    let wrapper: Wrapper = toml::from_str(&text).expect("shipped lifecycle.toml deserializes");
    let script = wrapper
        .session_lifecycle
        .script
        .into_iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("shipped lifecycle.toml carries {name}"));
    match script.teardown.expect("teardown exists") {
        LifecycleCommand::Shell(command) => command,
        LifecycleCommand::Builtin(id) => panic!("{name} ships a shell teardown, not builtin {id}"),
    }
}

/// A scratch fossil repository with a seeded trunk and a root worktree,
/// shaped like an autocoder run mid-flight.
struct ScratchRepo {
    root: PathBuf,
    repo_file: PathBuf,
    /// Path of trunk's leaf check-in, captured at seed time.
    trunk_leaf: String,
}

impl ScratchRepo {
    /// Creates the repo, seeds trunk with one check-in, forks the run
    /// branch, and opens the root worktree — the state after step 0 of
    /// an autocoder run.
    fn new(tag: &str) -> Self {
        let root = tempfile::tempdir().expect("tempdir").into_path().join(tag);
        std::fs::create_dir_all(&root).expect("mkdir");
        let repo_file = root.join("pipeline.fossil");
        fossil(&root, &["init", repo_file.to_str().expect("utf8 repo")]);
        let checkout = root.join("checkout");
        std::fs::create_dir_all(&checkout).expect("mkdir checkout");
        fossil(
            &checkout,
            &["open", repo_file.to_str().expect("utf8 repo"), "trunk"],
        );
        std::fs::write(checkout.join("seed.txt"), "seed\n").expect("seed file");
        fossil(&checkout, &["add", "seed.txt"]);
        fossil(&checkout, &["commit", "-m", "seed trunk"]);
        let trunk_leaf = leaf_of(&repo_file, "trunk");

        // The run branch: what the root script's setup produces.
        run_script(
            render(
                &shipped_setup("autocoder-fossil-root"),
                &[("dir", "worktrees/run-wt"), ("branch", "autocoder/scratch")],
            ),
            &root,
        );

        Self {
            root,
            repo_file,
            trunk_leaf,
        }
    }

    fn trunk_leaf_now(&self) -> String {
        leaf_of(&self.repo_file, "trunk")
    }

    fn branch_exists(&self, name: &str) -> bool {
        let out = Command::new("fossil")
            .args(["branch", "ls", "-R"])
            .arg(&self.repo_file)
            .output()
            .expect("fossil branch ls");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.trim() == name)
    }

    fn worktree_exists(&self, dir: &str) -> bool {
        self.root.join(dir).join(".fslckout").is_file()
    }
}

/// Runs `fossil` in `dir`, panicking on failure with its output.
fn fossil(dir: &Path, fossil_args: &[&str]) {
    let out = Command::new("fossil")
        .args(fossil_args)
        .current_dir(dir)
        .env("FOSSIL_USER", "tester")
        .output()
        .expect("fossil runs");
    assert!(
        out.status.success(),
        "fossil {fossil_args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The leaf check-in uuid of a branch: the highest-rid check-in
/// carrying the branch's `sym-` tag. Trunk's leaf only moves when a
/// commit or merge lands on trunk, which is exactly what the tests
/// count.
fn leaf_of(repo: &Path, branch: &str) -> String {
    let query = format!(
        "SELECT blob.uuid FROM tagxref JOIN tag ON tag.tagid=tagxref.tagid \
         JOIN blob ON blob.rid=tagxref.rid \
         WHERE tag.tagname='sym-{branch}' AND tagxref.tagtype>0 \
         ORDER BY tagxref.rid DESC LIMIT 1;"
    );
    let out = Command::new("fossil")
        .args(["sql", "-R"])
        .arg(repo)
        .arg(&query)
        .output()
        .expect("fossil sql runs");
    assert!(
        out.status.success(),
        "leaf query failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Named args in the workflow's spelling.
fn args(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// The tool's argument ordering, mirrored against the public template
/// API (identical logic to the crate's `order_args`, which is
/// `pub(crate)`).
fn order(template: &CommandTemplate, pairs: &BTreeMap<String, String>) -> Vec<String> {
    template
        .params()
        .iter()
        .filter_map(|p| match p {
            Param::Named(name) => pairs.get(name).cloned(),
            _ => None,
        })
        .collect()
}

/// Renders a raw script against named args through the real template
/// path — parse, order, render (which shell-quotes every value) — the
/// exact `run_lifecycle_script` path applies.
fn render(raw: &str, pairs: &[(&str, &str)]) -> String {
    let template = CommandTemplate::parse(raw);
    template.render(&order(&template, &args(pairs)))
}

/// Like [`run_script`], but expects refusal: returns the failure.
fn try_run(command: String, dir: &Path) -> Result<String, String> {
    let out = Command::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()))
        .arg("-c")
        .arg(&command)
        .current_dir(dir)
        .env("FOSSIL_USER", "tester")
        .output()
        .expect("script runs");
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// Executes an already-rendered command in `dir`.
fn run_script(command: String, dir: &Path) -> String {
    let out = Command::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned()))
        .arg("-c")
        .arg(&command)
        .current_dir(dir)
        .env("FOSSIL_USER", "tester")
        .output()
        .expect("script runs");
    assert!(
        out.status.success(),
        "rendered script failed: {command}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[rstest::rstest]
#[test]
fn root_setup_forks_run_branch_off_trunk_and_trunk_stands() {
    // Given a scratch repo with a seeded trunk.
    let repo = ScratchRepo::new("root-setup");

    // When the root setup has run (it did, in new()).
    // Then the run branch exists, its worktree is open, and trunk's
    // leaf is exactly the seed check-in.
    assert!(repo.branch_exists("autocoder/scratch"));
    assert!(repo.worktree_exists("worktrees/run-wt"));
    assert_eq!(
        repo.trunk_leaf_now(),
        repo.trunk_leaf,
        "root setup must not advance trunk"
    );
}

#[rstest::rstest]
#[test]
fn stage_lifecycle_forks_from_base_and_never_writes_trunk() {
    // Given a scratch repo with the run branch open.
    let repo = ScratchRepo::new("stage-lifecycle");

    // When a stage's full lifecycle runs: setup forks off the run
    // branch (under worktrees/), work happens, the finishing merge
    // lands on the run branch (the workflow's step, done from the
    // root worktree), then the inert teardown runs — closing nothing,
    // deleting nothing.
    let stdout = run_script(
        render(
            &shipped_setup("autocoder-fossil-stage"),
            &[
                ("dir", "worktrees/impl-wt"),
                ("branch", "autocoder/scratch/impl"),
                ("base", "autocoder/scratch"),
            ],
        ),
        &repo.root,
    );
    let stage_wt = repo.root.join("worktrees/impl-wt");
    assert!(
        stdout.contains(stage_wt.to_str().expect("utf8 wt")),
        "setup prints the worktree path for CWD adoption: {stdout}"
    );
    std::fs::write(stage_wt.join("work.txt"), "stage work\n").expect("stage file");
    fossil(&stage_wt, &["add", "work.txt"]);
    fossil(&stage_wt, &["commit", "-m", "stage work"]);
    fossil(
        &repo.root.join("worktrees/run-wt"),
        &["merge", "autocoder/scratch/impl"],
    );
    fossil(
        &repo.root.join("worktrees/run-wt"),
        &["commit", "-m", "merge stage into run branch"],
    );

    run_script(
        render(
            &shipped_teardown("autocoder-fossil-stage"),
            &[("dir", "worktrees/impl-wt")],
        ),
        &repo.root,
    );

    // Then the worktree is untouched by teardown — still an open
    // checkout, files intact — and trunk still stands at its seed
    // leaf.
    assert!(
        repo.worktree_exists("worktrees/impl-wt"),
        "teardown is inert: the checkout stays open for run-end cleanup"
    );
    assert!(stage_wt.join("work.txt").is_file(), "files are kept");
    assert!(
        repo.branch_exists("autocoder/scratch/impl"),
        "the stage branch is kept"
    );
    assert_eq!(
        repo.trunk_leaf_now(),
        repo.trunk_leaf,
        "no stage-lifecycle step may advance trunk"
    );
}

#[rstest::rstest]
#[test]
fn guarded_move_accepts_a_real_stage_worktree_into_the_trash() {
    // Given a scratch repo with a stage worktree under worktrees/.
    let repo = ScratchRepo::new("trash-accept");
    run_script(
        render(
            &shipped_setup("autocoder-fossil-stage"),
            &[
                ("dir", "worktrees/impl-wt"),
                ("branch", "autocoder/scratch/impl"),
                ("base", "autocoder/scratch"),
            ],
        ),
        &repo.root,
    );

    // When the run-end cleanup runs the guarded move.
    let stdout = run_script(
        render(
            &shipped_setup("autocoder-trash-stage"),
            &[("dir", "worktrees/impl-wt")],
        ),
        &repo.root,
    );

    // Then the worktree moved into the fixed trash, intact.
    assert!(
        stdout.contains("trashed"),
        "the move reports success: {stdout}"
    );
    assert!(
        !repo.root.join("worktrees/impl-wt").exists(),
        "the worktree left its dispatch path"
    );
    let trashed = repo.root.join(".jinn-autocoder-trash/impl-wt");
    assert!(
        trashed.join(".fslckout").is_file() && trashed.join("checkout").is_dir()
            || trashed.exists(),
        "the worktree sits in the trash, recoverable: {:?}",
        std::fs::read_dir(repo.root.join(".jinn-autocoder-trash"))
            .map(|d| d.flatten().map(|e| e.path()).collect::<Vec<_>>())
    );
    assert_eq!(
        repo.trunk_leaf_now(),
        repo.trunk_leaf,
        "the move never touches trunk"
    );
}

#[rstest::rstest]
#[test]
fn guarded_move_refuses_paths_outside_the_run_area() {
    // Given a scratch repo — no stage worktrees at all.
    let repo = ScratchRepo::new("trash-refuse");

    // A decoy: a directory that looks like a worktree path but holds
    // something else (here, a plain directory with a stray file, and
    // an innocent sibling that must survive whatever happens).
    let decoy = repo.root.join("worktrees/decoy");
    std::fs::create_dir_all(&decoy).expect("mkdir decoy");
    std::fs::write(decoy.join("innocent.txt"), "keep me\n").expect("decoy file");
    let bystander = repo.root.join("src");
    std::fs::create_dir_all(&bystander).expect("mkdir src");
    std::fs::write(bystander.join("lib.rs"), "pub fn x() {}\n").expect("src file");

    // When the guarded move is aimed at a non-checkout under
    // worktrees/, it must refuse — and leave everything in place.
    let failed = try_run(
        render(
            &shipped_setup("autocoder-trash-stage"),
            &[("dir", "worktrees/decoy")],
        ),
        &repo.root,
    );
    assert!(failed.is_err(), "a directory without .fslckout is refused");
    assert!(
        decoy.join("innocent.txt").is_file(),
        "the refused directory is untouched"
    );

    // And when aimed at a real checkout that sits outside worktrees/ —
    // the fixture's trunk `checkout/` is exactly that — it must refuse
    // too: the prefix gate fires even though the checkout marker gate
    // would pass.
    let escaped = try_run(
        render(
            &shipped_setup("autocoder-trash-stage"),
            &[("dir", "checkout")],
        ),
        &repo.root,
    );
    assert!(
        escaped.is_err(),
        "a real checkout outside worktrees/ must be refused"
    );
    assert!(
        repo.worktree_exists("checkout"),
        "the refused trunk checkout is untouched"
    );
    assert!(
        bystander.join("lib.rs").is_file() && !bystander.join(".jinn-autocoder-trash").exists(),
        "nothing outside the trash was moved or removed"
    );
    assert_eq!(
        repo.trunk_leaf_now(),
        repo.trunk_leaf,
        "refusals never touch trunk"
    );
}

#[rstest::rstest]
#[test]
fn purge_deletes_the_fixed_trash_and_nothing_else() {
    // Given a scratch repo with two trashed worktrees and a bystander.
    let repo = ScratchRepo::new("purge");
    for slug in ["a-wt", "b-wt"] {
        run_script(
            render(
                &shipped_setup("autocoder-fossil-stage"),
                &[
                    ("dir", &format!("worktrees/{slug}")),
                    ("branch", &format!("autocoder/scratch/{slug}")),
                    ("base", "autocoder/scratch"),
                ],
            ),
            &repo.root,
        );
        run_script(
            render(
                &shipped_setup("autocoder-trash-stage"),
                &[("dir", &format!("worktrees/{slug}"))],
            ),
            &repo.root,
        );
    }
    let bystander = repo.root.join("worktrees/still-here");
    std::fs::create_dir_all(&bystander).expect("mkdir bystander");
    std::fs::write(bystander.join("keep.txt"), "keep\n").expect("keep file");
    assert!(repo.root.join(".jinn-autocoder-trash").is_dir());

    // When the purge runs — the literal, placeholder-free command.
    let stdout = run_script(
        render(&shipped_setup("autocoder-trash-purge"), &[]),
        &repo.root,
    );

    // Then the trash is gone, everything else stands, and trunk is
    // exactly where it started.
    assert!(
        !repo.root.join(".jinn-autocoder-trash").exists(),
        "the trash directory is removed"
    );
    assert!(
        bystander.join("keep.txt").is_file(),
        "the bystander survives the purge"
    );
    assert!(
        repo.worktree_exists("worktrees/run-wt"),
        "the root worktree survives the purge"
    );
    assert_eq!(
        repo.trunk_leaf_now(),
        repo.trunk_leaf,
        "the purge never touches trunk"
    );
    let _ = stdout;
}

#[rstest::rstest]
#[test]
fn g2_merge_is_the_single_trunk_advance() {
    // Given a scratch repo with a run branch holding merged stage work.
    let repo = ScratchRepo::new("g2-merge");
    run_script(
        render(
            &shipped_setup("autocoder-fossil-stage"),
            &[
                ("dir", "worktrees/qa-wt"),
                ("branch", "autocoder/scratch/qa"),
                ("base", "autocoder/scratch"),
            ],
        ),
        &repo.root,
    );
    let stage_wt = repo.root.join("worktrees/qa-wt");
    std::fs::write(stage_wt.join("feature.txt"), "feature\n").expect("feature file");
    fossil(&stage_wt, &["add", "feature.txt"]);
    fossil(&stage_wt, &["commit", "-m", "feature"]);
    fossil(
        &repo.root.join("worktrees/run-wt"),
        &["merge", "autocoder/scratch/qa"],
    );
    fossil(
        &repo.root.join("worktrees/run-wt"),
        &["commit", "-m", "merge qa"],
    );

    // When the G2 merge runs — the run branch merged into trunk from a
    // trunk checkout, exactly once, as the workflow's QA-acceptance
    // step prescribes.
    let checkout = repo.root.join("checkout");
    fossil(&checkout, &["merge", "autocoder/scratch"]);
    fossil(
        &checkout,
        &["commit", "-m", "G2: merge run branch into trunk"],
    );

    // Then trunk advanced exactly once past its seed leaf, and holds
    // the stage's file.
    let after = repo.trunk_leaf_now();
    assert_ne!(after, repo.trunk_leaf, "G2 advanced trunk");
    assert!(
        checkout.join("feature.txt").is_file(),
        "trunk holds the merged stage work"
    );
    // The G2 merge commit has exactly two parents — the trunk line and
    // the run branch — so this was one merge commit, not a series of
    // trunk writes.
    let parent_query = format!(
        "SELECT count(*) FROM plink JOIN blob ON blob.rid=plink.cid WHERE blob.uuid='{after}';"
    );
    let out = Command::new("fossil")
        .args(["sql", "-R"])
        .arg(&repo.repo_file)
        .arg(&parent_query)
        .output()
        .expect("sql");
    let parents: u32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("parent count");
    assert_eq!(
        parents, 2,
        "the G2 merge commit has exactly two parents (trunk + run branch): one trunk advance"
    );
}
