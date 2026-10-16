//! Productive (productive.io): the open tasks assigned to you, over its JSON:API with `curl`.
//!
//! Config (`productive.*`): `organizationId` (required), `tokenEnv` (the env var holding the
//! API token, default `PRODUCTIVE_AUTH_TOKEN`), `apiUrl`, `appUrl`. Your person id and the
//! organisation's name come from `/organization_memberships`, so nothing else is configured.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::core::config;
use crate::core::tools::{slugify, split_http_status};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub org_id: String,
    pub token_env: String,
    pub api_url: String,
    pub app_url: String,
}

impl Settings {
    pub fn from_raw(raw: &Value) -> Result<Settings, String> {
        let p = &raw["productive"];
        let text = |key: &str, default: &str| {
            p[key]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(default)
                .trim_end_matches('/')
                .to_string()
        };
        let org_id = match &p["organizationId"] {
            Value::String(s) if !s.trim().is_empty() => s.trim().to_string(),
            Value::Number(n) => n.to_string(),
            _ => {
                return Err(
                    "set productive.organizationId: bearclaw config set productive.organizationId <id>"
                        .into(),
                );
            }
        };
        Ok(Settings {
            org_id,
            token_env: text("tokenEnv", "PRODUCTIVE_AUTH_TOKEN"),
            api_url: text("apiUrl", "https://api.productive.io/api/v2"),
            app_url: text("appUrl", "https://app.productive.io"),
        })
    }

    /// Read fresh from disk: a long-running dashboard sees config edits.
    pub fn load() -> Result<Settings, String> {
        Self::from_raw(&config::load_from(&config::path()).raw)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub number: String,
    pub title: String,
    pub project: String,
    pub status: String,
    /// `YYYY-MM-DD`.
    pub due_date: Option<String>,
    pub updated_at: String,
    pub url: String,
    /// The description as plain text.
    pub description: String,
    pub assignee_id: Option<String>,
    /// Where the status sits on the board: (category — not started, started, closed —, position).
    pub status_rank: (u8, i64),
}

fn rel_id<'a>(item: &'a Value, rel: &str) -> Option<&'a str> {
    item["relationships"][rel]["data"]["id"].as_str()
}

fn included<'a>(v: &'a Value, kind: &str, id: &str) -> Option<&'a Value> {
    v["included"]
        .as_array()?
        .iter()
        .find(|i| i["type"] == kind && i["id"] == id)
        .map(|i| &i["attributes"])
}

fn included_name<'a>(v: &'a Value, kind: &str, id: &str) -> Option<&'a str> {
    included(v, kind, id)?["name"].as_str()
}

/// `(person id, organisation name)` of your membership in `org_id`.
pub fn parse_membership(v: &Value, org_id: &str) -> Option<(String, String)> {
    let m = v["data"]
        .as_array()?
        .iter()
        .find(|m| rel_id(m, "organization") == Some(org_id))?;
    let person = rel_id(m, "person")?.to_string();
    let name = included_name(v, "organizations", org_id)
        .unwrap_or("")
        .to_string();
    Some((person, name))
}

/// Tasks with their project and workflow status joined from `included`.
pub fn parse_tasks(v: &Value, task_url_base: &str) -> Vec<Task> {
    let Some(data) = v["data"].as_array() else {
        return Vec::new();
    };
    data.iter()
        .map(|t| {
            let a = &t["attributes"];
            let s = |k: &str| a[k].as_str().unwrap_or("").to_string();
            let id = t["id"].as_str().unwrap_or("").to_string();
            let joined = |rel: &str, kind: &str| {
                rel_id(t, rel)
                    .and_then(|i| included_name(v, kind, i))
                    .unwrap_or("")
                    .to_string()
            };
            Task {
                number: match &a["number"] {
                    Value::Number(n) => n.to_string(),
                    other => other.as_str().unwrap_or("").to_string(),
                },
                title: s("title"),
                project: joined("project", "projects"),
                status: joined("workflow_status", "workflow_statuses"),
                due_date: a["due_date"].as_str().map(str::to_string),
                updated_at: s("updated_at"),
                url: format!("{task_url_base}/tasks/{id}"),
                description: html_to_text(a["description"].as_str().unwrap_or("")),
                assignee_id: rel_id(t, "assignee").map(str::to_string),
                status_rank: rel_id(t, "workflow_status")
                    .and_then(|i| included(v, "workflow_statuses", i))
                    .map_or((0, 0), |s| {
                        (
                            s["category_id"].as_u64().unwrap_or(0) as u8,
                            s["position"].as_i64().unwrap_or(0),
                        )
                    }),
                id,
            }
        })
        .collect()
}

/// Productive stores descriptions as HTML; this is enough of it to read in a terminal.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            rest = &rest[start..];
            break;
        };
        let tag = rest[start + 1..start + end].trim().to_ascii_lowercase();
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let closing = tag.starts_with('/');
        match name.as_str() {
            "br" => out.push('\n'),
            "li" if !closing => out.push_str("- "),
            "p" | "li" | "div" | "pre" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "tr"
                if closing =>
            {
                out.push('\n')
            }
            _ => {}
        }
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let mut text = String::new();
    let mut blank = 0;
    for line in decoded.lines().map(str::trim_end) {
        blank = if line.is_empty() { blank + 1 } else { 0 };
        if blank < 2 {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.trim().to_string()
}

/// Only the tasks really assigned to `person`: the API's assignee filter reads a search index
/// that lags, so a task handed to someone else can still come back for a while.
pub fn assigned_to(tasks: Vec<Task>, person: &str) -> Vec<Task> {
    tasks
        .into_iter()
        .filter(|t| t.assignee_id.as_deref() == Some(person))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grouping {
    /// By workflow status, in board order.
    Status,
    /// By project, alphabetically.
    Project,
}

/// Group names with the indices of their tasks (task order kept within a group).
pub fn group(tasks: &[Task], by: Grouping) -> Vec<(String, Vec<usize>)> {
    let mut groups: Vec<(String, (u8, i64), Vec<usize>)> = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        let (name, rank) = match by {
            Grouping::Status => (&t.status, t.status_rank),
            Grouping::Project => (&t.project, (0, 0)),
        };
        match groups.iter_mut().find(|g| &g.0 == name) {
            Some(g) => g.2.push(i),
            None => groups.push((name.clone(), rank, vec![i])),
        }
    }
    groups.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    groups.into_iter().map(|(n, _, idx)| (n, idx)).collect()
}

/// Due date ascending (no date last), then most recently updated first.
pub fn sort_tasks(tasks: &mut [Task]) {
    tasks.sort_by(|a, b| match (&a.due_date, &b.due_date) {
        (Some(x), Some(y)) if x != y => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        _ => b.updated_at.cmp(&a.updated_at),
    });
}

/// The first `errors[].detail` (or title) of a JSON:API error body, else the body itself.
fn error_text(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let e = &v["errors"][0];
    e["detail"]
        .as_str()
        .or(e["title"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(160).collect())
}

/// GET `path` (with its query) from the API. The token goes to curl on stdin, never argv.
fn api_get(s: &Settings, token: &str, path: &str) -> Result<Value, String> {
    let url = format!("{}{path}", s.api_url);
    let mut child = Command::new("curl")
        .args([
            "-sSg",
            "--max-time",
            "15",
            "-K",
            "-",
            "-w",
            "\n%{http_code}",
            &url,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("curl: no stdin")?;
        let q = |v: &str| v.replace('\\', "\\\\").replace('"', "\\\"");
        let cfg = format!(
            "header = \"X-Auth-Token: {}\"\nheader = \"X-Organization-Id: {}\"\nheader = \"Content-Type: application/vnd.api+json\"\n",
            q(token),
            q(&s.org_id)
        );
        stdin
            .write_all(cfg.as_bytes())
            .map_err(|e| format!("curl: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("Productive API unreachable: {}", err.trim()));
    }
    let (body, code) = split_http_status(&String::from_utf8_lossy(&out.stdout));
    if !(200..300).contains(&code) {
        return Err(format!("Productive API {code}: {}", error_text(&body)));
    }
    serde_json::from_str(&body).map_err(|e| format!("Productive API: bad JSON ({e})"))
}

/// The API leaves a membership's person linkage null unless it is included.
const MEMBERSHIPS_PATH: &str = "/organization_memberships?include=person,organization";

/// Open tasks filtered to an assignee (`{person}`), with what the rows show and the assignee
/// to double-check against.
const TASKS_QUERY: &str = "/tasks?filter[assignee_id]={person}&filter[status]=1&include=project,workflow_status,assignee&page[size]=200";

/// Your open tasks in the configured organisation, sorted by [`sort_tasks`].
pub fn my_open_tasks(s: &Settings) -> Result<Vec<Task>, String> {
    let token = std::env::var(&s.token_env)
        .ok()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| format!("no Productive token: set ${}", s.token_env))?;
    let m = api_get(s, &token, MEMBERSHIPS_PATH)?;
    let (person, org_name) = parse_membership(&m, &s.org_id)
        .ok_or_else(|| format!("this token has no membership in organization {}", s.org_id))?;
    let base = format!("{}/{}-{}", s.app_url, s.org_id, slugify(&org_name));
    let v = api_get(s, &token, &TASKS_QUERY.replace("{person}", &person))?;
    let mut tasks = assigned_to(parse_tasks(&v, &base), &person);
    sort_tasks(&mut tasks);
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_need_an_org() {
        let e = Settings::from_raw(&json!({})).unwrap_err();
        assert!(e.contains("productive.organizationId"), "{e}");
        let s = Settings::from_raw(&json!({"productive":{"organizationId":"42"}})).unwrap();
        assert_eq!(s.org_id, "42");
        assert_eq!(s.token_env, "PRODUCTIVE_AUTH_TOKEN");
        assert_eq!(s.api_url, "https://api.productive.io/api/v2");
        assert_eq!(s.app_url, "https://app.productive.io");
        // `bearclaw config set` stores a bare number as a number.
        let n =
            Settings::from_raw(&json!({"productive":{"organizationId":42,"tokenEnv":"MY_TOKEN"}}))
                .unwrap();
        assert_eq!(
            (n.org_id.as_str(), n.token_env.as_str()),
            ("42", "MY_TOKEN")
        );
    }

    #[test]
    fn membership_picks_the_configured_org() {
        let v = json!({
            "data": [
                {"id":"m1","type":"organization_memberships","relationships":{
                    "organization":{"data":{"type":"organizations","id":"1"}},
                    "person":{"data":{"type":"people","id":"p1"}}}},
                {"id":"m42","type":"organization_memberships","relationships":{
                    "organization":{"data":{"type":"organizations","id":"42"}},
                    "person":{"data":{"type":"people","id":"p42"}}}}
            ],
            "included": [
                {"id":"1","type":"organizations","attributes":{"name":"Other"}},
                {"id":"42","type":"organizations","attributes":{"name":"Acme Ltd."}}
            ]
        });
        assert_eq!(
            parse_membership(&v, "42"),
            Some(("p42".to_string(), "Acme Ltd.".to_string()))
        );
        assert_eq!(parse_membership(&v, "7"), None);
    }

    #[test]
    fn tasks_join_project_and_status() {
        let v = json!({
            "data": [{"id":"7","type":"tasks","attributes":{
                "number":"12","title":"Fix login","due_date":"2026-01-02",
                "updated_at":"2026-01-01T10:00:00.000+01:00",
                "description":"<p>Fix it &amp; ship</p><ul><li>one</li><li>two</li></ul>"},
              "relationships":{
                "project":{"data":{"type":"projects","id":"3"}},
                "workflow_status":{"data":{"type":"workflow_statuses","id":"9"}}}}],
            "included": [
                {"id":"3","type":"projects","attributes":{"name":"Website"}},
                {"id":"9","type":"workflow_statuses","attributes":{"name":"In progress"}}
            ]
        });
        let t = &parse_tasks(&v, "https://app.example/42-acme-ltd")[0];
        assert_eq!(t.number, "12");
        assert_eq!(t.title, "Fix login");
        assert_eq!(t.project, "Website");
        assert_eq!(t.status, "In progress");
        assert_eq!(t.due_date.as_deref(), Some("2026-01-02"));
        assert_eq!(t.url, "https://app.example/42-acme-ltd/tasks/7");
        assert_eq!(t.description, "Fix it & ship\n- one\n- two");
    }

    // The API leaves `relationships.person.data` null unless the person is included.
    #[test]
    fn memberships_are_fetched_with_their_person() {
        assert!(MEMBERSHIPS_PATH.contains("include=person,organization"));
    }

    #[test]
    fn tasks_assigned_elsewhere_are_dropped() {
        let v = json!({
            "data": [
                {"id":"1","type":"tasks","attributes":{"title":"mine"},
                 "relationships":{"assignee":{"data":{"type":"people","id":"p42"}},
                   "workflow_status":{"data":{"type":"workflow_statuses","id":"9"}}}},
                {"id":"2","type":"tasks","attributes":{"title":"moved on"},
                 "relationships":{"assignee":{"data":{"type":"people","id":"p7"}}}},
                {"id":"3","type":"tasks","attributes":{"title":"unassigned"},
                 "relationships":{"assignee":{"data":null}}}
            ],
            "included": [{"id":"9","type":"workflow_statuses",
                          "attributes":{"name":"Working","category_id":2,"position":4}}]
        });
        let tasks = parse_tasks(&v, "https://app.example/x");
        assert_eq!(tasks[0].assignee_id.as_deref(), Some("p42"));
        assert_eq!(tasks[0].status_rank, (2, 4));
        let mine = assigned_to(tasks, "p42");
        let ids: Vec<&str> = mine.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["1"]);
        // The search index lags: a task API returns for the filter can be someone else's now.
        assert!(TASKS_QUERY.contains("include=project,workflow_status,assignee"));
    }

    #[test]
    fn groups_follow_the_board_order() {
        let t = |id: &str, status: &str, rank: (u8, i64), project: &str| Task {
            id: id.into(),
            status: status.into(),
            status_rank: rank,
            project: project.into(),
            ..Default::default()
        };
        let tasks = vec![
            t("a", "Working", (2, 4), "Web"),
            t("b", "Triage", (1, 1), "Api"),
            t("c", "Peer Review", (2, 6), "Web"),
            t("d", "Working", (2, 4), "Api"),
        ];
        let by_status: Vec<(String, Vec<usize>)> = group(&tasks, Grouping::Status);
        assert_eq!(
            by_status,
            vec![
                ("Triage".to_string(), vec![1]),
                ("Working".to_string(), vec![0, 3]),
                ("Peer Review".to_string(), vec![2]),
            ]
        );
        let by_project = group(&tasks, Grouping::Project);
        assert_eq!(
            by_project,
            vec![
                ("Api".to_string(), vec![1, 3]),
                ("Web".to_string(), vec![0, 2])
            ]
        );
    }

    #[test]
    fn overdue_first_then_recent() {
        let t = |id: &str, due: Option<&str>, updated: &str| Task {
            id: id.into(),
            due_date: due.map(Into::into),
            updated_at: updated.into(),
            ..Default::default()
        };
        let mut v = vec![
            t("a", Some("2026-01-02"), "1"),
            t("b", None, "9"),
            t("c", None, "5"),
            t("d", Some("2026-01-01"), "1"),
        ];
        sort_tasks(&mut v);
        let ids: Vec<&str> = v.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["d", "a", "b", "c"]);
    }
}
