//! Completing a cited `path:lines` into a code anchor.
//!
//! A model cites code the way it sees it: a path and a range of lines. An
//! anchor needs what the model has no way to know — the commit the lines
//! are pinned to, those lines as they are at that commit, and their digest
//! ([`CodeAnchor`]) — and the hook has exactly that: the working tree, and
//! the repository under it. This module turns one into the other.
//!
//! What cannot be reproduced is refused, not anchored: lines that are not
//! committed, in a tree the record cannot point back to, would make the
//! anchor a claim about code that never existed (decision
//! `01M2TMXDNTTF68HGNG8XKTGM0K`). Refusals ride the hook's visible line, so
//! each says what to do next, not which git command failed.
//!
//! Every lookup runs from the *file's* directory, which is what makes a
//! submodule right without special casing: git answers with the nearest
//! repository — the submodule's own root and HEAD — for what lives inside
//! it.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

use converge_client::{CodeAnchor, EXCERPT_LINES};
use serde_json::Value;

/// A code citation as the model wrote it: a place, and no commit.
pub struct Cited {
    pub path: String,
    pub lines: (u32, u32),
}

/// How a citation turned out: the anchor to send, or why it could not be
/// one, phrased for the person who cited the code.
pub enum Resolved {
    Anchor(CodeAnchor),
    Refused(String),
}

/// Complete the bare code citations in a `decision_add`'s arguments, in
/// place, and return the reasons for the ones that could not be completed
/// (the caller puts them on the visible line).
///
/// An item that already names a commit is a finished anchor — an importer
/// or the model may send one — and passes through untouched. What is left
/// incomplete is the caller's to drop: a half anchor the server would
/// reject is worth less than nothing.
pub fn complete(merged: &mut Value, cwd: &Path) -> Vec<String> {
    let Some(items) = merged
        .get("code_evidence")
        .and_then(Value::as_array)
        .cloned()
    else {
        return Vec::new();
    };
    let mut refusals = Vec::new();
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        match (item["commit"].is_null(), cited(&item)) {
            (true, Some(citation)) => match resolve(cwd, &citation) {
                Resolved::Anchor(anchor) => {
                    kept.push(serde_json::to_value(anchor).expect("an anchor is plain data"))
                }
                Resolved::Refused(why) => refusals.push(why),
            },
            _ => kept.push(item),
        }
    }
    merged["code_evidence"] = Value::Array(kept);
    refusals
}

/// The citation shape the resolver can work on: a path, two line numbers,
/// and no commit yet. Anything else is not ours to complete.
fn cited(item: &Value) -> Option<Cited> {
    let path = item.get("path")?.as_str()?;
    let lines = item.get("lines")?.as_array()?;
    if lines.len() != 2 {
        return None;
    }
    let (start, end) = (lines[0].as_u64()?, lines[1].as_u64()?);
    Some(Cited {
        path: path.to_string(),
        lines: (u32::try_from(start).ok()?, u32::try_from(end).ok()?),
    })
}

/// Complete one citation against the repository under `cwd`: the full
/// anchor, or the reason it stands refused.
pub fn resolve(cwd: &Path, cited: &Cited) -> Resolved {
    let (start, end) = cited.lines;
    let here = format!("{}:{start}-{end}", cited.path);
    // The limits first: they need no repository, and a citation that
    // breaks them is one no anchor could hold.
    if start == 0 || end < start {
        return Resolved::Refused(format!(
            "`{here}` is not a line range — cite `start-end`, 1-based and inclusive"
        ));
    }
    let count = end - start + 1;
    if count as usize > EXCERPT_LINES {
        return Resolved::Refused(format!(
            "`{here}` cites {count} lines — split it ({EXCERPT_LINES} per anchor)"
        ));
    }

    // A citation is read from where the model stands...
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let seen = joined(&cwd, &cited.path);
    let refusal = match anchor_at(&seen, start, end, &here) {
        Ok(anchor) => return Resolved::Anchor(anchor),
        Err(why) => why,
    };
    // ...and if nothing is there, from the repository root, which is how
    // a path out of a subdirectory is often meant.
    if let Some(root) = root_of(&cwd)
        && let at_root = joined(&root, &cited.path)
        && at_root != seen
        && let Ok(anchor) = anchor_at(&at_root, start, end, &here)
    {
        return Resolved::Anchor(anchor);
    }
    Resolved::Refused(refusal)
}

/// The citation read as naming `at`: find the repository that owns it,
/// take the cited lines from its HEAD, and check that the working tree
/// still reads the same there.
fn anchor_at(at: &Path, start: u32, end: u32, here: &str) -> Result<CodeAnchor, String> {
    let (root, commit) = repository(at).ok_or_else(|| match root_of(at) {
        // A repository that has nothing in it yet: the citation is fine,
        // the commit is what is missing.
        Some(_) => format!("`{here}` is not committed yet — commit it first"),
        None => format!("`{here}` is not inside a git repository — commit it in one first"),
    })?;
    // The anchor carries the path as the repository sees it, which is not
    // the way the model wrote it.
    let rel = match at.strip_prefix(&root) {
        Ok(rel) => relative(rel),
        Err(_) => return Err(format!("`{here}` is outside `{}`", root.display())),
    };
    if rel.is_empty() || at.is_dir() {
        return Err(format!("`{here}` names a directory — cite a file"));
    }
    // The excerpt comes from the commit, never from the working tree: a
    // record has to say what the commit says.
    let blob = match git(&root, &["show", &format!("{commit}:{rel}")]) {
        Some(blob) => blob,
        None => return Err(unanchored(at, &root, &rel, here)),
    };
    let at_commit = String::from_utf8(blob)
        .map_err(|_| format!("`{here}` is not a text file — cite lines of text"))?;
    let lines = at_commit.lines().collect::<Vec<_>>();
    let Some(picked) = lines.get(start as usize - 1..end as usize) else {
        return Err(format!(
            "`{here}` is past the end of `{rel}` — {} line(s) at `{commit}`, cite a range inside it",
            lines.len()
        ));
    };
    // Reproducibility: the same range on disk has to read the same, or the
    // anchor would claim lines the commit does not hold. Only the cited
    // range counts — an edit elsewhere in the file leaves it standing.
    let on_disk = std::fs::read_to_string(at).ok();
    let on_disk = on_disk
        .as_deref()
        .map(|text| text.lines().collect::<Vec<_>>());
    if on_disk
        .as_deref()
        .and_then(|l| l.get(start as usize - 1..end as usize))
        != Some(picked)
    {
        return Err(format!("uncommitted changes in `{here}` — commit first"));
    }

    let excerpt = format!("{}\n", picked.join("\n"));
    let digest = CodeAnchor::digest_of(&excerpt);
    let anchor = CodeAnchor {
        commit,
        path: rel,
        lines: (start, end),
        excerpt,
        digest,
        // Server-set once the repository has been asked.
        verified_at: None,
        mismatch: None,
    };
    // Storage has the last word on what an anchor is; refuse here rather
    // than send one it would send back.
    anchor
        .validate()
        .map(|_| anchor)
        .map_err(|e| format!("`{here}` is not a citable anchor — {e}"))
}

/// `git show` found nothing at that path in `commit`: say which of the
/// three cases it is — staged, untracked, or simply not there.
fn unanchored(at: &Path, root: &Path, rel: &str, here: &str) -> String {
    if git(root, &["ls-files", "--error-unmatch", "--", rel]).is_some() {
        format!("`{here}` is not committed yet — commit it first")
    } else if at.exists() {
        format!("`{here}` is not tracked — commit first")
    } else {
        format!("`{here}` is not in the repository — check the path, or commit the file first")
    }
}

/// The repository that owns `path` and its HEAD commit, looked up from the
/// file's own directory so the nearest repository wins.
fn repository(path: &Path) -> Option<(PathBuf, String)> {
    let out = String::from_utf8(git(
        standing_dir(path),
        &["rev-parse", "--show-toplevel", "HEAD"],
    )?)
    .ok()?;
    let mut found = out.lines();
    let root = PathBuf::from(found.next()?.trim());
    let commit = found.next()?.trim().to_string();
    // A full sha or nothing: the anchor's `commit` is 40-hex by contract.
    (commit.len() == 40 && root.is_dir()).then_some((root, commit))
}

/// The root of the repository `path` stands in, without its commit.
fn root_of(path: &Path) -> Option<PathBuf> {
    let out =
        String::from_utf8(git(standing_dir(path), &["rev-parse", "--show-toplevel"])?).ok()?;
    let root = PathBuf::from(out.lines().next()?.trim());
    root.is_dir().then_some(root)
}

/// The nearest directory that exists at or above `path`: every lookup
/// starts from a directory, and a citation may name one that is not there.
fn standing_dir(path: &Path) -> &Path {
    let mut dir = path.parent().unwrap_or(path);
    while !dir.is_dir() {
        dir = match dir.parent() {
            Some(parent) => parent,
            None => break,
        };
    }
    dir
}

/// `path` seen from `base`, absolute and folded. Purely lexical — which
/// reading names a real committed file is git's answer, not the
/// filesystem's.
fn joined(base: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    normalize(&joined)
}

/// Fold `.` away and `..` into the component it pops; a `..` with nothing
/// left to pop stands, as it does in a git pathspec.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir if !out.pop() => out.push(part),
            part => out.push(part),
        }
    }
    out
}

/// Repository-relative, forward slashes — the form an anchor carries.
fn relative(path: &Path) -> String {
    path.iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Run git in `dir`, capturing stdout; `None` when it could not run or
/// answered with an error.
fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

/// A temporary repository with `files` committed. Canonicalized, so the
/// path a test hands the resolver is the path git reports as its root.
/// Crate-visible: `hook`'s wiring test cites a file from one too.
#[cfg(test)]
pub(crate) fn test_repo(files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cvg-evidence-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let dir = std::fs::canonicalize(dir).unwrap();
    assert!(git(&dir, &["init", "-q"]).is_some(), "git init failed");
    for (path, body) in files {
        let at = dir.join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, body).unwrap();
    }
    commit(&dir);
    dir
}

#[cfg(test)]
fn commit(dir: &Path) {
    assert!(git(dir, &["add", "-A"]).is_some());
    assert!(
        git(
            dir,
            &[
                "-c",
                "user.email=tests@converge",
                "-c",
                "user.name=Tests",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ]
        )
        .is_some(),
        "git commit failed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "one\ntwo\nthree\nfour\nfive\n";

    fn head(dir: &Path) -> String {
        String::from_utf8(git(dir, &["rev-parse", "HEAD"]).unwrap())
            .unwrap()
            .trim()
            .to_string()
    }

    fn citation(path: &str, lines: (u32, u32)) -> Cited {
        Cited {
            path: path.to_string(),
            lines,
        }
    }

    /// The anchor, panicking on a refusal so the asserts read as one shape.
    fn anchored(resolved: Resolved) -> CodeAnchor {
        match resolved {
            Resolved::Anchor(anchor) => anchor,
            Resolved::Refused(why) => panic!("expected an anchor, refused: {why}"),
        }
    }

    fn refused(resolved: Resolved) -> String {
        match resolved {
            Resolved::Refused(why) => why,
            Resolved::Anchor(anchor) => panic!("expected a refusal, anchored: {anchor:?}"),
        }
    }

    #[test]
    fn a_clean_citation_becomes_an_anchor_at_head() {
        let repo = test_repo(&[("src/lib.rs", BODY)]);
        let anchor = anchored(resolve(&repo, &citation("src/lib.rs", (2, 3))));
        assert_eq!(anchor.commit, head(&repo));
        assert_eq!(anchor.path, "src/lib.rs");
        assert_eq!(anchor.lines, (2, 3));
        assert_eq!(anchor.excerpt, "two\nthree\n");
        assert_eq!(anchor.digest, CodeAnchor::digest_of("two\nthree\n"));
        anchor.validate().unwrap();
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn an_uncommitted_edit_inside_the_cited_range_is_refused() {
        let repo = test_repo(&[("src/lib.rs", BODY)]);
        std::fs::write(repo.join("src/lib.rs"), "one\nTWO\nthree\nfour\nfive\n").unwrap();
        let why = refused(resolve(&repo, &citation("src/lib.rs", (2, 3))));
        assert!(
            why.contains("uncommitted changes") && why.contains("commit first"),
            "{why}"
        );
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn an_uncommitted_edit_outside_the_cited_range_still_anchors() {
        let repo = test_repo(&[("src/lib.rs", BODY)]);
        std::fs::write(repo.join("src/lib.rs"), "one\ntwo\nthree\nfour\nFIVE\n").unwrap();
        let anchor = anchored(resolve(&repo, &citation("src/lib.rs", (2, 3))));
        // The commit's lines, not the working tree's — and the digest of
        // exactly those.
        assert_eq!(anchor.excerpt, "two\nthree\n");
        anchor.validate().unwrap();
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn an_untracked_file_is_refused() {
        let repo = test_repo(&[("src/lib.rs", BODY)]);
        std::fs::write(repo.join("src/new.rs"), "a\nb\n").unwrap();
        let why = refused(resolve(&repo, &citation("src/new.rs", (1, 2))));
        assert!(
            why.contains("not tracked") && why.contains("commit first"),
            "{why}"
        );
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn a_range_that_is_not_a_range_is_refused_with_the_matching_reason() {
        let repo = test_repo(&[("src/lib.rs", BODY)]);
        let past = refused(resolve(&repo, &citation("src/lib.rs", (4, 99))));
        assert!(past.contains("past the end"), "{past}");
        let back = refused(resolve(&repo, &citation("src/lib.rs", (3, 2))));
        assert!(back.contains("1-based"), "{back}");
        let zero = refused(resolve(&repo, &citation("src/lib.rs", (0, 2))));
        assert!(zero.contains("1-based"), "{zero}");
        let long = refused(resolve(
            &repo,
            &citation("src/lib.rs", (1, EXCERPT_LINES as u32 + 1)),
        ));
        assert!(long.contains("split it"), "{long}");
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn a_citation_from_a_subdirectory_carries_the_repository_path() {
        let repo = test_repo(&[("src/lib.rs", BODY)]);
        let anchor = anchored(resolve(&repo.join("src"), &citation("lib.rs", (1, 1))));
        assert_eq!(anchor.path, "src/lib.rs");
        assert_eq!(anchor.commit, head(&repo));
        assert_eq!(anchor.excerpt, "one\n");
        anchor.validate().unwrap();
        std::fs::remove_dir_all(&repo).ok();
    }

    #[test]
    fn a_file_in_a_submodule_anchors_in_the_submodule() {
        let repo = test_repo(&[("README.md", "readme\n")]);
        let inner = test_repo(&[("g.rs", "g1\ng2\n")]);
        // A submodule is a clone, and git refuses file:// clones unless
        // the protocol is allowed for the call.
        assert!(
            git(
                &repo,
                &[
                    "-c",
                    "protocol.file.allow=always",
                    "submodule",
                    "add",
                    "-q",
                    &inner.to_string_lossy(),
                    "sub",
                ]
            )
            .is_some(),
            "submodule add failed"
        );
        let anchor = anchored(resolve(&repo, &citation("sub/g.rs", (1, 2))));
        // The submodule's repository: its root for the path, its HEAD for
        // the commit.
        assert_eq!(anchor.path, "g.rs");
        assert_eq!(anchor.commit, head(&repo.join("sub")));
        assert_ne!(anchor.commit, head(&repo));
        assert_eq!(anchor.excerpt, "g1\ng2\n");
        anchor.validate().unwrap();
        std::fs::remove_dir_all(&repo).ok();
        std::fs::remove_dir_all(&inner).ok();
    }
}
