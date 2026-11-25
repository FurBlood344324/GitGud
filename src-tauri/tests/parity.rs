//! State parity against the real `git` binary.
//!
//! The backend is a parser over git's output, and its unit tests check that
//! parse against fixed expectations. This file asks the other question: it
//! drives a scratch repository through the backend's own functions, reads the
//! same state a second time straight from the `git` CLI, and compares the two
//! independent reads.
//!
//! What gets compared are canonical facts, never raw porcelain text: sets of
//! (path, status) pairs, sets of commit hashes with their subjects and
//! parents, counts. A git release may reflow its output and that must not
//! paint this lab red, while a parse that drifts from git must.
//!
//! Two honest gaps. Rename sources (the backend parses `orig_path`) are not
//! compared yet: no seeded state produces a rename status record, and a slice
//! that wants one must carry the old paths into `Facts` on both sides first.
//! A rename commit does exist in the history slices, where `file_log` is asked
//! to cross it, but that compares a commit, not a status record.
//! And the CLI-side read below is a fresh parse, not a call into the backend's
//! parser, but a systematic misread of porcelain would still look identical
//! on both sides; each test also asserts against a fixed expectation written
//! down by hand, and that is what catches a shared delusion.
//!
//! Every test is `#[ignore]`d, so `cargo test` stays fast and the per-push
//! gate never runs this. See docs/TESTING.md.

use gitgud_lib::git;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// The real git binary

/// Runs `git` in `dir` and returns its stdout, or panics with git's stderr.
///
/// The environment is inherited on purpose: the backend runs git the same way,
/// so a machine-level setting that would break one breaks both and a parity
/// failure means the parse, not the setup. What the fixture pins (identity,
/// signing, hooks, excludes) lives in the repository's own config, which
/// outranks the global one.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = run_git(dir, args, &[]);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// [`git`] with extra environment variables, for a fixture that has to pin
/// timestamps: search results are ordered by author time.
fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let out = run_git(dir, args, env);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// [`git`] where a failure is the answer rather than a panicking test: for the
/// questions whose answer is "git says no such ref, no such file".
fn git_try(dir: &Path, args: &[&str]) -> Option<String> {
    let out = run_git(dir, args, &[]);
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run_git(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .envs(env.iter().copied())
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("the git CLI has to be on PATH: this lab is a comparison with it")
}

// ---------------------------------------------------------------------------
// The fixture

/// A throwaway repository, removed when the test ends, panic or not.
struct Scratch {
    dir: PathBuf,
}

/// Keeps two tests in one process off each other's directory.
static NEXT: AtomicU32 = AtomicU32::new(0);

impl Scratch {
    /// An empty repository on `main`, with a fixed identity and the settings
    /// that make a fixture reproducible on any machine.
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "gitgud-parity-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let repo = Scratch { dir };

        repo.git(&["init", "-q", "-b", "main"]);
        // An excludes file of our own, so a global one can't hide a file from
        // one reader and not the other; no hooks, so neither runs behind our
        // back. Both point at paths that don't exist, which is what git wants.
        let excludes = repo.dir.join("no-global-excludes").display().to_string();
        for (key, value) in [
            ("user.name", "Parity Lab"),
            ("user.email", "parity@example.com"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
            ("core.autocrlf", "false"),
            ("core.hooksPath", "no-hooks"),
            ("core.excludesfile", &excludes),
        ] {
            repo.git(&["config", key, value]);
        }
        repo
    }

    /// The same repository with three fixed commits and a known file set:
    /// README.md, docs/guide.md, src/lib.rs, src/main.rs, tests/smoke.rs.
    fn seeded(name: &str) -> Scratch {
        let repo = Scratch::new(name);
        repo.write("README.md", "# scratch\n");
        repo.write("src/main.rs", "fn main() {}\n");
        repo.commit("seed the repository");

        repo.write("docs/guide.md", "read me first\n");
        repo.write("src/lib.rs", "pub fn answer() -> u32 {\n    42\n}\n");
        repo.commit("add a guide");

        repo.write("README.md", "# scratch\n\nstill scratch\n");
        repo.write("tests/smoke.rs", "#[test]\nfn smoke() {}\n");
        repo.commit("write the smoke test");
        repo
    }

    fn repo(&self) -> &Path {
        &self.dir
    }

    fn git(&self, args: &[&str]) -> String {
        git(&self.dir, args)
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.dir.join(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Stages everything and commits through the CLI, so the fixture state is
    /// made by git itself and not by the code under test.
    fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    /// [`Scratch::commit`] at a pinned author and committer time, since the
    /// search order is author time; `author` overrides the fixture identity.
    /// A message with a blank line in it gets a body, like `-m` twice would.
    fn commit_at(&self, message: &str, when: &str, author: Option<&str>) {
        self.git(&["add", "-A"]);
        let stamp = format!("{when} +0000");
        let mut args = vec!["commit", "-q", "-m", message];
        if let Some(author) = author {
            args.extend(["--author", author]);
        }
        let env = [
            ("GIT_AUTHOR_DATE", stamp.as_str()),
            ("GIT_COMMITTER_DATE", stamp.as_str()),
        ];
        git_env(&self.dir, &args, &env);
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

// ---------------------------------------------------------------------------
// Two reads of one state, reduced to the same facts

/// What a status read says, in a shape both sides can produce: which paths are
/// staged and with what, which are changed in the working tree, which git has
/// never seen, which are mid-conflict.
#[derive(Debug, Default, PartialEq, Eq)]
struct Facts {
    staged: BTreeSet<(String, char)>,
    unstaged: BTreeSet<(String, char)>,
    untracked: BTreeSet<String>,
    conflicted: BTreeSet<String>,
}

impl Facts {
    /// Records one path's index and worktree status, the way porcelain spells
    /// them: '.' is "unchanged".
    fn record(&mut self, path: &str, index: char, worktree: char) {
        if index != '.' {
            self.staged.insert((path.to_string(), index));
        }
        if worktree != '.' {
            self.unstaged.insert((path.to_string(), worktree));
        }
    }

    /// Read one: the backend's status, as sets.
    fn from_backend(repo: &Path) -> Facts {
        let status = git::status(repo).expect("the backend should read a scratch repository");
        let mut facts = Facts::default();
        for file in status.files {
            if file.untracked {
                facts.untracked.insert(file.path);
            } else if file.conflicted {
                facts.conflicted.insert(file.path);
            } else {
                // file.orig_path is parsed by the backend but deliberately not
                // compared in this slice; see the module docs.
                facts.record(&file.path, file.index, file.worktree);
            }
        }
        facts
    }

    /// Read two: `git status --porcelain=v2 -z`, parsed here from git's own
    /// record types rather than through the backend's parser.
    fn from_cli(repo: &Path) -> Facts {
        let raw = git(
            repo,
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
        );
        let mut entries = raw.split('\0').filter(|entry| !entry.is_empty());
        let mut facts = Facts::default();
        while let Some(entry) = entries.next() {
            // The record's second field is XY, the index and worktree status.
            let xy = |field: &str| {
                let mut chars = field.chars();
                (chars.next().unwrap_or('.'), chars.next().unwrap_or('.'))
            };
            match entry.as_bytes()[0] {
                // 1 XY sub mH mI mW hH hI path
                b'1' => {
                    let f: Vec<&str> = entry.splitn(9, ' ').collect();
                    if f.len() == 9 {
                        let (index, worktree) = xy(f[1]);
                        facts.record(f[8], index, worktree);
                    }
                }
                // 2 XY sub mH mI mW hH hI Xscore path, then the old path as
                // its own record. The rename source is not compared yet: no
                // seeded state renames, and a rename slice must add it to
                // Facts on both sides.
                b'2' => {
                    let f: Vec<&str> = entry.splitn(10, ' ').collect();
                    entries.next();
                    if f.len() == 10 {
                        let (index, worktree) = xy(f[1]);
                        facts.record(f[9], index, worktree);
                    }
                }
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                b'u' => {
                    let f: Vec<&str> = entry.splitn(11, ' ').collect();
                    if f.len() == 11 {
                        facts.conflicted.insert(f[10].to_string());
                    }
                }
                // ? path
                b'?' => {
                    facts.untracked.insert(entry[2..].to_string());
                }
                _ => {}
            }
        }
        facts
    }
}

/// The whole point: two reads of one state, compared, with a failure that
/// names the state they disagree about. The CLI's read comes back, so a test
/// can also assert what the state should be.
fn parity(repo: &Path, state: &str) -> Facts {
    let backend = Facts::from_backend(repo);
    let cli = Facts::from_cli(repo);
    assert_eq!(
        backend, cli,
        "the backend and the git CLI disagree about {state}: backend on the left, git on the right"
    );
    cli
}

fn pairs<const N: usize>(items: [(&str, char); N]) -> BTreeSet<(String, char)> {
    items
        .into_iter()
        .map(|(path, status)| (path.to_string(), status))
        .collect()
}

fn names<const N: usize>(items: [&str; N]) -> BTreeSet<String> {
    items.into_iter().map(str::to_string).collect()
}

/// `rev-parse` for one name, trimmed: the hash a spec points at.
fn rev(repo: &Path, spec: &str) -> String {
    git(repo, &["rev-parse", spec]).trim().to_string()
}

/// The checked-out branch's name, as git abbreviates it.
fn current_branch(repo: &Path) -> String {
    git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string()
}

// ---------------------------------------------------------------------------
// Reads the branch, amend and history slices share

/// One branch as either side sees it: its name, whether it is checked out, and
/// its upstream if it tracks one.
type BranchFact = (String, bool, Option<String>);

fn named(name: &str, current: bool) -> BranchFact {
    (name.to_string(), current, None)
}

fn tracking(name: &str, current: bool, upstream: &str) -> BranchFact {
    (name.to_string(), current, Some(upstream.to_string()))
}

/// Read one: the backend's branch list, as a set.
fn branches_from_backend(repo: &Path) -> BTreeSet<BranchFact> {
    git::branches(repo)
        .expect("the backend should list the branches of a scratch repository")
        .into_iter()
        .map(|branch| (branch.name, branch.current, branch.upstream))
        .collect()
}

/// Read two: the names `git branch` prints, the one `rev-parse` calls current,
/// and each branch's own `@{upstream}`, asked one at a time.
fn branches_from_cli(repo: &Path) -> BTreeSet<BranchFact> {
    let current = current_branch(repo);
    git(repo, &["branch", "--list", "--format=%(refname:short)"])
        .lines()
        .filter(|line| !line.is_empty())
        .map(|name| {
            let upstream = git_try(
                repo,
                &["rev-parse", "--abbrev-ref", &format!("{name}@{{upstream}}")],
            );
            (name.to_string(), name == current, upstream)
        })
        .collect()
}

fn branches_parity(repo: &Path, state: &str) -> BTreeSet<BranchFact> {
    let backend = branches_from_backend(repo);
    let cli = branches_from_cli(repo);
    assert_eq!(
        backend, cli,
        "the backend and the git CLI disagree about the branches {state}: backend on the left, git on the right"
    );
    cli
}

/// Hash, parents and subject of every commit `git log <args>` prints, in the
/// order it printed them, parsed here rather than through the backend.
fn cli_history(repo: &Path, args: &[&str]) -> Vec<(String, Vec<String>, String)> {
    let mut all = vec!["log", "--format=%H%x1f%P%x1f%s%x1e"];
    all.extend_from_slice(args);
    let raw = git(repo, &all);
    raw.split('\x1e')
        .filter_map(|record| {
            let mut f = record.trim_start_matches('\n').split('\x1f');
            let hash = f.next().filter(|h| !h.is_empty())?.to_string();
            let parents = f.next()?.split_whitespace().map(String::from).collect();
            let subject = f.next()?.to_string();
            Some((hash, parents, subject))
        })
        .collect()
}

fn backend_history(commits: &[git::Commit]) -> Vec<(String, Vec<String>, String)> {
    commits
        .iter()
        .map(|commit| {
            (
                commit.hash.clone(),
                commit.parents.clone(),
                commit.subject.clone(),
            )
        })
        .collect()
}

/// The CLI's answer to the question `search_log` asks: the commit a hex query
/// is a prefix of, plus case-insensitive fixed-string matches in the message
/// and in the author, across every ref, newest first, capped at `limit`.
fn cli_search(repo: &Path, query: &str, limit: u32) -> Vec<(String, String)> {
    fn read(repo: &Path, limit: u32, args: &[&str]) -> Vec<(String, i64, String)> {
        let limit = format!("-n{limit}");
        let mut all = vec![
            "log",
            "--date-order",
            &limit,
            "--format=%H%x1f%at%x1f%s%x1e",
        ];
        all.extend_from_slice(args);
        let raw = git(repo, &all);
        raw.split('\x1e')
            .filter_map(|record| {
                let mut f = record.trim_start_matches('\n').split('\x1f');
                let hash = f.next().filter(|h| !h.is_empty())?.to_string();
                let time: i64 = f.next()?.parse().ok()?;
                let subject = f.next()?.to_string();
                Some((hash, time, subject))
            })
            .collect()
    }

    let mut found = Vec::new();
    let is_hex = query.len() >= 4 && query.chars().all(|c| c.is_ascii_hexdigit());
    if is_hex {
        let spec = format!("{query}^{{commit}}");
        if git_try(repo, &["rev-parse", "-q", "--verify", &spec]).is_some() {
            found.extend(read(repo, 1, &[&spec]));
        }
    }
    for field in ["--grep", "--author"] {
        let pattern = format!("{field}={query}");
        let mut args = vec!["-i", "-F", &pattern];
        args.extend(["--branches", "--remotes", "--tags", "HEAD"]);
        found.extend(read(repo, limit, &args));
    }
    let mut seen = BTreeSet::new();
    found.retain(|(hash, _, _)| seen.insert(hash.clone()));
    found.sort_by_key(|(_, time, _)| std::cmp::Reverse(*time));
    found.truncate(limit as usize);
    found
        .into_iter()
        .map(|(hash, _, subject)| (hash, subject))
        .collect()
}

fn search_parity(repo: &Path, query: &str, limit: u32, state: &str) -> Vec<(String, String)> {
    let backend = git::search_log(repo, query, limit)
        .expect("the backend should search the history of a scratch repository");
    let backend_facts: Vec<(String, String)> = backend
        .into_iter()
        .map(|commit| (commit.hash, commit.subject))
        .collect();
    let cli = cli_search(repo, query, limit);
    assert_eq!(
        backend_facts, cli,
        "the backend and the git CLI disagree about the search for '{query}' ({state}): backend on the left, git on the right"
    );
    cli
}

/// One file's history, read twice: the CLI side uses the same literal pathspec
/// magic the backend builds, so a name with pattern characters is compared as
/// a name and not as a pattern. What is compared is each commit's hash, its
/// parents and its subject, in the order git walked the file.
fn file_log_parity(
    repo: &Path,
    path: &str,
    limit: u32,
    state: &str,
) -> Vec<(String, Vec<String>, String)> {
    let backend = git::file_log(repo, path, limit)
        .expect("the backend should read one file's history in a scratch repository");
    let limit = format!("-n{limit}");
    let spec = format!(":(literal){path}");
    let cli = cli_history(repo, &["--follow", "HEAD", &limit, "--", &spec]);
    let backend_facts = backend_history(&backend);
    assert_eq!(
        backend_facts, cli,
        "the backend and the git CLI disagree about the history of {state}: backend on the left, git on the right"
    );
    cli
}

// ---------------------------------------------------------------------------
// Tests: opt-in, and each one runs the real git binary

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn fresh_repo_status_parity() {
    let repo = Scratch::seeded("status");

    // A clean tree: every set is empty, read twice.
    let clean = parity(repo.repo(), "a clean tree");
    assert_eq!(clean, Facts::default());

    // The same question about the repository itself.
    let info = git::open(repo.repo()).unwrap();
    assert_eq!(
        Path::new(&info.path).canonicalize().unwrap(),
        repo.repo().canonicalize().unwrap()
    );
    assert_eq!(
        info.name,
        repo.repo().file_name().unwrap().to_string_lossy()
    );
    let status = git::status(repo.repo()).unwrap();
    assert_eq!(
        status.branch.as_deref(),
        Some(git(repo.repo(), &["rev-parse", "--abbrev-ref", "HEAD"]).trim())
    );

    // One edit, one deletion, one file git has never seen.
    repo.write("README.md", "# scratch\n\nedited by hand\n");
    fs::remove_file(repo.repo().join("docs/guide.md")).unwrap();
    repo.write("notes.txt", "a thought\n");

    let dirty = parity(repo.repo(), "a dirty tree");
    assert_eq!(dirty.staged, BTreeSet::new());
    assert_eq!(
        dirty.unstaged,
        pairs([("README.md", 'M'), ("docs/guide.md", 'D')])
    );
    assert_eq!(dirty.untracked, names(["notes.txt"]));
    assert_eq!(dirty.conflicted, BTreeSet::new());
}

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn unborn_branch_status_parity() {
    // Nothing committed yet: HEAD points nowhere, and staging has no HEAD to
    // restore from, which is a separate path on both sides.
    let repo = Scratch::new("unborn");
    repo.write("first.txt", "hello\n");

    let fresh = parity(repo.repo(), "an unborn branch");
    assert_eq!(fresh.untracked, names(["first.txt"]));
    assert_eq!(fresh.staged, BTreeSet::new());
    assert!(git::log(repo.repo(), 10).unwrap().is_empty());

    git::stage(repo.repo(), &["first.txt".to_string()]).unwrap();
    let staged = parity(repo.repo(), "a file staged before the first commit");
    assert_eq!(staged.staged, pairs([("first.txt", 'A')]));
    assert!(staged.untracked.is_empty());

    git::unstage(repo.repo(), &["first.txt".to_string()]).unwrap();
    let unstaged = parity(repo.repo(), "a file unstaged before the first commit");
    assert_eq!(unstaged.untracked, names(["first.txt"]));
}

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn stage_parity() {
    let repo = Scratch::seeded("stage");
    repo.write("README.md", "# scratch\n\nstaged on purpose\n");
    repo.write("src/lib.rs", "pub fn answer() -> u32 {\n    43\n}\n");
    repo.write("notes.txt", "untracked\n");

    // The backend stages one file; git says what that did to the index.
    git::stage(repo.repo(), &["README.md".to_string()]).unwrap();
    let one = parity(repo.repo(), "one file staged through the backend");
    assert_eq!(one.staged, pairs([("README.md", 'M')]));
    assert_eq!(one.unstaged, pairs([("src/lib.rs", 'M')]));
    assert_eq!(one.untracked, names(["notes.txt"]));

    // Then the rest, including a file git had never seen.
    git::stage(
        repo.repo(),
        &["src/lib.rs".to_string(), "notes.txt".to_string()],
    )
    .unwrap();
    let all = parity(repo.repo(), "everything staged through the backend");
    assert_eq!(
        all.staged,
        pairs([("README.md", 'M'), ("src/lib.rs", 'M'), ("notes.txt", 'A')])
    );
    assert!(all.unstaged.is_empty());
    assert!(all.untracked.is_empty());
}

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn commit_parity() {
    let repo = Scratch::seeded("commit");
    let parent = git::log(repo.repo(), 1).unwrap()[0].hash.clone();

    repo.write("src/lib.rs", "pub fn answer() -> u32 {\n    7\n}\n");
    git::stage(repo.repo(), &["src/lib.rs".to_string()]).unwrap();
    git::commit(repo.repo(), "the answer is seven").unwrap();

    // The backend's HEAD and git's HEAD are the same commit.
    let head = git(repo.repo(), &["rev-parse", "HEAD"]).trim().to_string();
    let log = git::log(repo.repo(), 1).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].hash, head);
    assert_eq!(log[0].subject, "the answer is seven");
    assert_eq!(log[0].parents, vec![parent]);

    // Its tree holds the file, at the content that was staged, and the tree
    // is clean afterwards.
    let tree = git(repo.repo(), &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(tree.lines().any(|line| line == "src/lib.rs"));
    assert_eq!(
        git(repo.repo(), &["show", "HEAD:src/lib.rs"]),
        "pub fn answer() -> u32 {\n    7\n}\n"
    );
    let settled = parity(repo.repo(), "a tree committed through the backend");
    assert_eq!(settled, Facts::default());
}

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn history_parity() {
    let repo = Scratch::seeded("history");
    let backend = git::log(repo.repo(), 100).unwrap();

    // The same refs, in the same order, read straight from git.
    let raw = git(
        repo.repo(),
        &[
            "log",
            "--date-order",
            "-n100",
            "--branches",
            "--remotes",
            "--tags",
            "HEAD",
            "--format=%H%x1f%P%x1f%s%x1e",
        ],
    );
    let cli: Vec<(String, Vec<String>, String)> = raw
        .split('\x1e')
        .filter_map(|record| {
            let mut f = record.trim_start_matches('\n').split('\x1f');
            let hash = f.next().filter(|h| !h.is_empty())?.to_string();
            let parents = f.next()?.split_whitespace().map(String::from).collect();
            let subject = f.next()?.to_string();
            Some((hash, parents, subject))
        })
        .collect();

    // Hash, parents and subject, position by position: the two reads were
    // asked for the same order, so a shuffled history fails here too.
    let backend_facts: Vec<(&str, Vec<&str>, &str)> = backend
        .iter()
        .map(|commit| {
            (
                commit.hash.as_str(),
                commit.parents.iter().map(String::as_str).collect(),
                commit.subject.as_str(),
            )
        })
        .collect();
    let cli_facts: Vec<(&str, Vec<&str>, &str)> = cli
        .iter()
        .map(|(hash, parents, subject)| {
            (
                hash.as_str(),
                parents.iter().map(String::as_str).collect(),
                subject.as_str(),
            )
        })
        .collect();
    assert_eq!(
        backend_facts, cli_facts,
        "the backend's history and git's log disagree"
    );

    // And the counts, which are the cheapest fact to say out loud.
    assert_eq!(backend.len(), 3);
    assert_eq!(cli.len(), 3);
    assert_eq!(
        git(repo.repo(), &["rev-list", "--count", "HEAD"]).trim(),
        "3"
    );
    let distinct: BTreeSet<&str> = cli.iter().map(|(hash, _, _)| hash.as_str()).collect();
    assert_eq!(distinct.len(), 3);
}

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn amend_parity() {
    let repo = Scratch::seeded("amend");
    let before = rev(repo.repo(), "HEAD");
    let tree = rev(repo.repo(), "HEAD^{tree}");
    let parent = rev(repo.repo(), "HEAD^");

    // Nothing staged, a new message: the same tree and the same parent, a new
    // commit, and the backend reads the message back.
    git::amend(repo.repo(), "seed the repository, spelled right").unwrap();
    let amended = rev(repo.repo(), "HEAD");
    assert_ne!(before, amended, "amending has to write a new commit");
    assert_eq!(
        git(repo.repo(), &["log", "-1", "--format=%B"]).trim_end(),
        "seed the repository, spelled right"
    );
    let last = git::last_commit(repo.repo()).unwrap().unwrap();
    assert_eq!(last.message, "seed the repository, spelled right");
    assert!(!last.pushed, "a fresh scratch repository has no remote");
    assert_eq!(rev(repo.repo(), "HEAD^{tree}"), tree, "the tree stands");
    assert_eq!(
        rev(repo.repo(), "HEAD^"),
        parent,
        "amending the tip keeps its parent"
    );
    assert_eq!(
        git(repo.repo(), &["rev-list", "--count", "HEAD"]).trim(),
        "3"
    );
    assert_eq!(
        parity(repo.repo(), "a tree amended with nothing staged"),
        Facts::default()
    );

    // Something staged this time: the amend swallows it into the last commit.
    repo.write(
        "src/main.rs",
        "fn main() {\n    println!(\"amended\");\n}\n",
    );
    git::stage(repo.repo(), &["src/main.rs".to_string()]).unwrap();
    git::amend(repo.repo(), "amend era").unwrap();
    let once = rev(repo.repo(), "HEAD");
    assert_ne!(once, amended);
    assert_eq!(
        git(repo.repo(), &["show", "HEAD:src/main.rs"]),
        "fn main() {\n    println!(\"amended\");\n}\n"
    );
    assert_eq!(
        rev(repo.repo(), "HEAD^"),
        parent,
        "an amend replaces one commit, it does not stack"
    );
    assert_eq!(
        git(repo.repo(), &["rev-list", "--count", "HEAD"]).trim(),
        "3"
    );
    assert_eq!(
        parity(repo.repo(), "a tree amended with a staged change"),
        Facts::default()
    );

    // A body survives the round trip, and the two reads of it agree.
    git::amend(repo.repo(), "amend era\n\nwith the body git writes").unwrap();
    let message = git::last_commit(repo.repo()).unwrap().unwrap().message;
    assert_eq!(
        message,
        git(repo.repo(), &["log", "-1", "--format=%B"]).trim_end()
    );
    assert_eq!(
        message, "amend era\n\nwith the body git writes",
        "the body is part of the fact, not decoration"
    );

    // Refusals: an empty message, and amending with no commit to amend.
    let settled = rev(repo.repo(), "HEAD");
    assert!(git::amend(repo.repo(), "  ").is_err());
    assert_eq!(
        rev(repo.repo(), "HEAD"),
        settled,
        "a refused amend changes nothing"
    );

    let unborn = Scratch::new("amend-unborn");
    assert!(git::amend(unborn.repo(), "nothing to amend").is_err());
    assert!(git::last_commit(unborn.repo()).unwrap().is_none());
}

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn branch_parity() {
    let repo = Scratch::seeded("branch");
    // A remote to track: nothing is fetched, the configuration is what git
    // reads when it is asked for a branch's upstream.
    repo.git(&["config", "remote.origin.url", "nowhere.invalid/scratch"]);
    repo.git(&[
        "config",
        "remote.origin.fetch",
        "+refs/heads/*:refs/remotes/origin/*",
    ]);
    let tip = rev(repo.repo(), "HEAD");
    repo.git(&["update-ref", "refs/remotes/origin/main", &tip]);
    repo.git(&["config", "branch.main.remote", "origin"]);
    repo.git(&["config", "branch.main.merge", "refs/heads/main"]);

    // One branch, checked out, tracking what it tracks.
    let one = branches_parity(repo.repo(), "with one branch");
    assert_eq!(one, BTreeSet::from([tracking("main", true, "origin/main")]));

    // A branch created through the backend is checked out and named what git
    // calls it.
    git::create_branch(repo.repo(), "topic").unwrap();
    let two = branches_parity(repo.repo(), "after creating a branch");
    assert_eq!(
        two,
        BTreeSet::from([tracking("main", false, "origin/main"), named("topic", true),]),
        "a branch is born with no upstream, and the one it left keeps its own"
    );
    assert_eq!(
        git(repo.repo(), &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "topic"
    );
    assert_eq!(
        repo.git(&["status", "--porcelain=v2", "--branch"])
            .lines()
            .find_map(|line| line.strip_prefix("# branch.head ")),
        Some("topic")
    );

    // A commit on it, so it has work main does not.
    repo.write("docs/topic.md", "the topic starts here\n");
    repo.commit_at("the topic starts", "1700000000", None);
    let topic_tip = rev(repo.repo(), "HEAD");
    assert_eq!(
        branches_parity(repo.repo(), "after a commit on the new branch"),
        BTreeSet::from([tracking("main", false, "origin/main"), named("topic", true),])
    );

    // Deleting it without force is refused, and says so rather than erroring.
    // Only once something else is checked out: git refuses the checked-out
    // branch for a different reason, and this asks about merge state.
    git::switch_branch(repo.repo(), "main").unwrap();
    let refused = git::delete_branch(repo.repo(), "topic", false).unwrap();
    assert!(refused.not_merged, "an unmerged branch must not go quietly");
    assert!(
        git_try(repo.repo(), &["rev-parse", "--verify", "refs/heads/topic"]).is_some(),
        "a refused delete leaves the branch where it was"
    );
    let untouched = branches_parity(repo.repo(), "after a refused delete");
    assert_eq!(
        untouched,
        BTreeSet::from([tracking("main", true, "origin/main"), named("topic", false),])
    );

    // Back on the topic branch, and given something to track: renaming the
    // checked-out branch keeps it checked out, and keeps its upstream.
    git::switch_branch(repo.repo(), "topic").unwrap();
    repo.git(&["config", "branch.topic.remote", "origin"]);
    repo.git(&["config", "branch.topic.merge", "refs/heads/main"]);
    git::rename_branch(repo.repo(), "topic", "topic-renamed").unwrap();
    let renamed = branches_parity(repo.repo(), "after renaming the current branch");
    assert_eq!(
        renamed,
        BTreeSet::from([
            tracking("main", false, "origin/main"),
            tracking("topic-renamed", true, "origin/main"),
        ])
    );
    assert_eq!(rev(repo.repo(), "refs/heads/topic-renamed"), topic_tip);

    // Back to main, then a branch that is already merged: that one goes
    // without force, and main's own tip does not move.
    git::switch_branch(repo.repo(), "main").unwrap();
    assert_eq!(
        branches_parity(repo.repo(), "after switching back"),
        BTreeSet::from([
            tracking("main", true, "origin/main"),
            tracking("topic-renamed", false, "origin/main"),
        ])
    );
    git::create_branch(repo.repo(), "already-home").unwrap();
    git::switch_branch(repo.repo(), "main").unwrap();
    let merged = git::delete_branch(repo.repo(), "already-home", false).unwrap();
    assert!(!merged.not_merged);
    assert!(git_try(
        repo.repo(),
        &["rev-parse", "--verify", "refs/heads/already-home"]
    )
    .is_none());

    // And the forced delete of the branch with work on it.
    let forced = git::delete_branch(repo.repo(), "topic-renamed", true).unwrap();
    assert!(!forced.not_merged);
    assert!(git_try(
        repo.repo(),
        &["rev-parse", "--verify", "refs/heads/topic-renamed"]
    )
    .is_none());
    let alone = branches_parity(repo.repo(), "after both deletes");
    assert_eq!(
        alone,
        BTreeSet::from([tracking("main", true, "origin/main")])
    );
    assert_eq!(rev(repo.repo(), "HEAD"), tip, "main never moved");

    // What git itself says about what is merged, asked independently.
    let not_merged = git(repo.repo(), &["branch", "--no-merged", "main"]);
    assert!(
        not_merged.trim().is_empty(),
        "nothing is left unmerged, but git lists: {not_merged}"
    );
}

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn search_parity_for_subject_author_and_hash() {
    let repo = Scratch::seeded("search");
    // Three more commits with pinned times, so the order is the author time
    // and not the second the test ran: one body-only match for "kettle", one
    // that a regex would match for "1.0" and a fixed string must not, and one
    // by another author.
    repo.write("docs/kettle.md", "put the kettle on\n");
    repo.commit_at("ship v1x0 of the kettle", "1700000000", None);
    repo.write("docs/books.md", "the ledger\n");
    repo.commit_at(
        "audit the books\n\nthe boilerplate mentions the kettle",
        "1700000100",
        None,
    );
    repo.write("docs/authors.md", "grace was here\n");
    repo.commit_at(
        "note the author",
        "1700000200",
        Some("Grace Hopper <grace@example.com>"),
    );
    let tip = rev(repo.repo(), "HEAD");
    let subjects = |facts: &[(String, String)]| -> Vec<String> {
        facts.iter().map(|(_, subject)| subject.clone()).collect()
    };

    // A word in one subject and one body, newest first. The newest match is
    // the audit commit; HEAD, which carries the other author, does not mention
    // the kettle at all.
    let kettle = search_parity(repo.repo(), "kettle", 10, "a message search");
    assert_eq!(
        subjects(&kettle),
        ["audit the books", "ship v1x0 of the kettle"]
    );
    assert_eq!(
        kettle[0].0,
        rev(repo.repo(), "HEAD^"),
        "the newest match comes first"
    );

    // Case does not matter on either side.
    let shouted = search_parity(repo.repo(), "KETTLE", 10, "an uppercased search");
    assert_eq!(shouted, kettle);

    // Fixed string, not a pattern: "1.0" as a regex would match "v1x0".
    let dotted = search_parity(repo.repo(), "1.0", 10, "a query with a dot in it");
    assert!(
        dotted.is_empty(),
        "a fixed-string search must not match v1x0: {dotted:?}"
    );

    // By author, as a substring and ignoring case.
    let author = search_parity(repo.repo(), "grace", 10, "an author search");
    assert_eq!(subjects(&author), ["note the author"]);

    // By hash: the full one and a prefix long enough to be read as one.
    let by_hash = search_parity(repo.repo(), &tip, 10, "a full hash search");
    assert_eq!(
        by_hash.first().map(|(hash, _)| hash.as_str()),
        Some(tip.as_str())
    );
    let prefix = &tip[..7];
    let by_prefix = search_parity(repo.repo(), prefix, 10, "a hash prefix search");
    assert_eq!(
        by_prefix.first().map(|(hash, _)| hash.as_str()),
        Some(tip.as_str())
    );

    // The cap is the same cap on both sides, and the order is the same order.
    let capped = search_parity(repo.repo(), "the", 2, "a capped search");
    assert_eq!(capped.len(), 2);
    let all = search_parity(repo.repo(), "the", 10, "an uncapped search");
    assert_eq!(all.len(), 5, "five of the six subjects contain it");
    assert!(
        capped.iter().all(|fact| all.contains(fact)),
        "a capped search may only return matches the uncapped one also returns"
    );

    // No match is an empty list, not an error, and the empty query returns
    // nothing at all rather than everything.
    assert!(search_parity(repo.repo(), "zebra", 10, "a search with no match").is_empty());
    assert!(git::search_log(repo.repo(), "   ", 10).unwrap().is_empty());
}

#[test]
#[ignore = "the parity lab: see docs/TESTING.md, run with cargo test --test parity -- --ignored"]
fn file_log_parity_across_a_rename() {
    let repo = Scratch::seeded("file-log");
    // docs/guide.md moves, and the new name gets one more commit.
    repo.git(&["mv", "docs/guide.md", "docs/handbook.md"]);
    repo.commit_at("the guide moves house", "1700000000", None);
    let moved = rev(repo.repo(), "HEAD");
    repo.write("docs/handbook.md", "read me first\n\nthen read me again\n");
    repo.commit_at("the guide gets a second page", "1700000100", None);
    let second = rev(repo.repo(), "HEAD");

    // Following the new name walks back through the rename to where the file
    // was born, and the two reads agree hash for hash.
    let history = file_log_parity(repo.repo(), "docs/handbook.md", 100, "a file that moved");
    let subjects: Vec<&str> = history
        .iter()
        .map(|(_, _, subject)| subject.as_str())
        .collect();
    assert_eq!(
        subjects,
        [
            "the guide gets a second page",
            "the guide moves house",
            "add a guide"
        ]
    );
    assert_eq!(history[0].0, second);
    assert_eq!(history[1].0, moved);
    assert_eq!(history.len(), 3);
    assert_eq!(
        history[1].1,
        vec![rev(repo.repo(), "HEAD~2")],
        "the rename commit's parent came with the fact"
    );

    // The old name still has its own history, which stops at the rename.
    let old = file_log_parity(repo.repo(), "docs/guide.md", 100, "a file that moved away");
    assert_eq!(old.len(), 2);
    assert_eq!(old, history[1..].to_vec());

    // A limit is honoured on both sides.
    let one = file_log_parity(repo.repo(), "docs/handbook.md", 1, "a file read once");
    assert_eq!(one, vec![history[0].clone()]);

    // A file nobody touched since the first commit: the walk stops early.
    let untouched = file_log_parity(repo.repo(), "src/main.rs", 100, "a file nobody touched");
    assert_eq!(
        untouched
            .iter()
            .map(|(_, _, s)| s.as_str())
            .collect::<Vec<_>>(),
        ["seed the repository"]
    );

    // Pattern characters in a name are a name, not a pattern: the literal
    // pathspec keeps the file with a star in its name apart from the one next
    // to it, which has its own commit right after.
    repo.write("docs/notes*.md", "a file with a star in its name\n");
    repo.commit_at("a file with a star in its name", "1700000200", None);
    let star = rev(repo.repo(), "HEAD");
    repo.write("docs/notesX.md", "a neighbour file\n");
    repo.commit_at("a neighbour file", "1700000300", None);
    let star_only = file_log_parity(repo.repo(), "docs/notes*.md", 100, "a name with a star");
    assert_eq!(
        star_only
            .iter()
            .map(|(h, _, s)| (h.as_str(), s.as_str()))
            .collect::<Vec<_>>(),
        [(star.as_str(), "a file with a star in its name")],
        "the star is a literal character in the name, not a pattern"
    );
    let neighbour = file_log_parity(repo.repo(), "docs/notesX.md", 100, "a neighbouring name");
    assert_eq!(
        neighbour
            .iter()
            .map(|(_, _, s)| s.as_str())
            .collect::<Vec<_>>(),
        ["a neighbour file"]
    );

    // A path git has never seen is an empty history, not an error.
    let never = file_log_parity(repo.repo(), "docs/never-written.md", 100, "a missing file");
    assert!(never.is_empty());
    assert!(git::file_log(repo.repo(), "", 10).is_err());
}
