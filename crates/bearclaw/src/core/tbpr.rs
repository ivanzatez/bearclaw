//! tb-pr as the PRs tab's source: when the `tb-pr` binary is around, the board is exactly its
//! board — its columns, its rules (direct review requests only; an approval takes a PR off your
//! list until someone asks again), its Productive task links — read from `tb-pr list --json`.

use std::process::{Command, Stdio};

use serde_json::Value;

use crate::core::prs::{Column, Pr, Rotting, Snapshot, View};
use crate::core::tools::find_binary;

/// `prs.tbPr` (a path) from the config, else `tb-pr` on the PATH.
pub fn binary() -> Option<String> {
    let raw = crate::core::config::load().raw.clone();
    raw["prs"]["tbPr"]
        .as_str()
        .map(str::to_string)
        .or_else(|| find_binary("tb-pr").map(|p| p.display().to_string()))
}

/// A Productive task's page: `<app>/<org slug>/tasks/<id>`.
pub fn task_url(app: &str, org_slug: &str, id: &str) -> String {
    format!("{}/{org_slug}/tasks/{id}", app.trim_end_matches('/'))
}

fn ms(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or(0, |d| d.timestamp_millis())
}

fn rotting(v: &Value) -> Rotting {
    match v.as_str() {
        Some("warming") => Rotting::Warming,
        Some("stale") => Rotting::Stale,
        Some("rotting") => Rotting::Rotting,
        Some("critical") => Rotting::Critical,
        _ => Rotting::Fresh,
    }
}

/// `https://github.com/<owner>/<repo>/pull/<n>` → owner.
fn owner_of(url: &str) -> String {
    url.trim_start_matches("https://github.com/")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn pr(column: Column, p: &Value) -> Pr {
    let s = |k: &str| p[k].as_str().unwrap_or("").to_string();
    let url = s("url");
    let created = ms(&p["created_at"]);
    Pr {
        column,
        owner: owner_of(&url),
        repo: s("repo"),
        number: p["number"].as_u64().unwrap_or(0),
        title: s("title"),
        author: s("author"),
        draft: p["state"] == "draft",
        created_at: created,
        // tb-pr ages a PR from its creation; so does the card.
        updated_at: created,
        additions: 0,
        deletions: 0,
        changed_files: 0,
        size: s("size"),
        base_branch: s("base_branch"),
        head_branch: s("head_branch"),
        cross_repository: false,
        head_oid: None,
        review_decision: None,
        checks: p["check_state"].as_str().map(str::to_string),
        comments: p["comments_count"].as_u64().unwrap_or(0),
        requested_reviewers: Vec::new(),
        my_review: None,
        mention: None,
        rotting: rotting(&p["rotting"]),
        local: None,
        productive_task_id: p["productive_task_id"].as_str().map(str::to_string),
        changes_requested_by: p["changes_requested_by"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        url,
    }
}

/// tb-pr's `list --json` board as a bearclaw snapshot. Notifications become the mentions lane.
pub fn parse(v: &Value) -> Snapshot {
    let mut prs = Vec::new();
    for (key, column) in [
        ("draft_mine", Column::DraftMine),
        ("review_mine", Column::ReviewMine),
        ("ready_to_merge_mine", Column::ReadyToMergeMine),
        ("waiting_on_me", Column::WaitingOnMe),
        ("waiting_on_author", Column::WaitingOnAuthor),
    ] {
        for p in v["columns"][key].as_array().into_iter().flatten() {
            prs.push(pr(column, p));
        }
    }
    for n in v["columns"]["notifications"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let mut p = pr(Column::UnseenMentions, n);
        p.owner = n["owner"].as_str().unwrap_or("").to_string();
        p.number = n["pr_number"].as_u64().unwrap_or(0);
        p.title = n["pr_title"].as_str().unwrap_or("").to_string();
        p.url = n["pr_url"].as_str().unwrap_or("").to_string();
        p.created_at = ms(&n["updated_at"]);
        p.updated_at = p.created_at;
        prs.push(p);
    }
    Snapshot {
        version: 1,
        user: v["user"].as_str().unwrap_or("").to_string(),
        scope: "tb-pr".into(),
        fetched_at: ms(&v["fetched_at"]),
        truncated: false,
        prs,
    }
}

/// The board from `bin`: `tb-pr refresh` first when `refresh`, then `tb-pr list --json` (which
/// serves tb-pr's own cache while it is fresh).
pub fn fetch(bin: &str, refresh: bool) -> Result<View, String> {
    if refresh {
        let st = Command::new(bin)
            .arg("refresh")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("tb-pr: {e}"))?;
        if !st.success() {
            return Err("tb-pr refresh failed".into());
        }
    }
    let out = Command::new(bin)
        .args(["list", "--json"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("tb-pr: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "tb-pr list: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("tb-pr list: {e}"))?;
    Ok(View {
        snap: parse(&v),
        stale: false,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tb_pr_columns_become_bearclaw_prs() {
        let v = json!({
            "user": "me",
            "fetched_at": "2026-10-19T12:11:29Z",
            "columns": {
                "draft_mine": [{"number": 7, "repo": "api", "title": "draft one",
                    "url": "https://github.com/acme/api/pull/7", "author": "me", "state": "draft",
                    "created_at": "2026-10-01T10:00:00Z", "age_days": 18.0, "size": "S",
                    "rotting": "stale", "comments_count": 2, "productive_task_id": "123",
                    "base_branch": "main", "head_branch": "x"}],
                "waiting_on_me": [{"number": 9, "repo": "web", "title": "review me",
                    "url": "https://github.com/acme/web/pull/9", "author": "dev", "state": "open",
                    "created_at": "2026-10-18T10:00:00Z", "age_days": 1.0, "rotting": "fresh",
                    "comments_count": 0, "changes_requested_by": ["lead"], "check_state": "failure"}],
                "waiting_on_author": [],
                "notifications": [{"thread_id": "t1", "reason": "mention", "owner": "acme",
                    "repo": "api", "pr_number": 3, "pr_title": "someone pinged you",
                    "pr_url": "https://github.com/acme/api/pull/3",
                    "updated_at": "2026-10-19T09:00:00Z", "age_days": 0.1}]
            }
        });
        let snap = parse(&v);
        assert_eq!(snap.user, "me");
        assert_eq!(snap.prs.len(), 3);
        let draft = snap.prs.iter().find(|p| p.number == 7).unwrap();
        assert_eq!(draft.column, Column::DraftMine);
        assert!(draft.draft);
        assert_eq!(draft.owner, "acme");
        assert_eq!(draft.rotting, Rotting::Stale);
        assert_eq!(draft.comments, 2);
        assert_eq!(draft.productive_task_id.as_deref(), Some("123"));
        let review = snap.prs.iter().find(|p| p.number == 9).unwrap();
        assert_eq!(review.column, Column::WaitingOnMe);
        assert!(!review.draft);
        assert_eq!(review.changes_requested_by, vec!["lead".to_string()]);
        assert_eq!(review.checks.as_deref(), Some("failure"));
        let mention = snap.prs.iter().find(|p| p.number == 3).unwrap();
        assert_eq!(mention.column, Column::UnseenMentions);
        assert_eq!(mention.title, "someone pinged you");
    }

    #[test]
    fn task_links_use_the_productive_org() {
        assert_eq!(
            task_url("https://app.example", "42-acme", "123"),
            "https://app.example/42-acme/tasks/123"
        );
    }
}
