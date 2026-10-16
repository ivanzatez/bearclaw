// `bearclaw todo` and `bearclaw sticky` against temp state files and a canned session
// (BEARCLAW_FIXTURE): a session's own item and note, and the errors outside a session.
#![allow(deprecated)]

mod common;

use common::{Env, stderr, stdout};

const SID: &str = "11111111-2222-3333-4444-555555555555";

fn setup() -> Env {
    let env = Env::new();
    let sessions = serde_json::json!([{
        "pid": 42, "session_id": SID, "name": "app-9d", "cwd": "/tmp", "status": "idle",
        "backend": "tmux", "handle": "%99", "tmux_session": "bearclaw-test-todo",
        "name_source": "user", "title": "fix the login loop"
    }]);
    common::fixture(env.dir.path(), &sessions.to_string());
    env
}

fn cmd(env: &Env) -> assert_cmd::Command {
    let mut c = env.cmd();
    c.env(
        "BEARCLAW_FIXTURE",
        env.dir.path().join("bearclaw-fixture.json"),
    )
    .env("BEARCLAW_TODO_FILE", env.path("todo.json"))
    .env("BEARCLAW_STICKY_DIR", env.path("sticky"))
    .env("HOME", env.dir.path())
    .timeout(std::time::Duration::from_secs(20));
    c
}

fn items(env: &Env) -> Vec<serde_json::Value> {
    let out = cmd(env).args(["todo", "list", "--json"]).output().unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()["items"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn todo_round_trip_through_the_cli() {
    let env = setup();
    let out = cmd(&env)
        .args(["todo", "set", "Permission error"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(stderr(&out).contains("--session"), "{}", stderr(&out));

    assert!(
        cmd(&env)
            .args(["todo", "add", "Look at Tini's PR"])
            .output()
            .unwrap()
            .status
            .success()
    );
    let out = cmd(&env)
        .args([
            "todo",
            "set",
            "Permission",
            "error",
            "--task",
            "https://app.example/tasks/7",
            "--session",
            "app-9d",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let all = items(&env);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0]["status"], "open");
    assert_eq!(all[1]["text"], "Permission error");
    assert_eq!(all[1]["status"], "doing");
    assert_eq!(all[1]["session"], SID);
    assert_eq!(all[1]["sessionLabel"], "app-9d");
    assert_eq!(all[1]["taskUrl"], "https://app.example/tasks/7");

    assert!(
        cmd(&env)
            .args(["todo", "done", "--session", "app-9d"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(items(&env)[1]["status"], "done");
    assert!(
        cmd(&env)
            .args(["todo", "claim", "1", "--session", "app-9d"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(items(&env)[0]["status"], "doing");
    let out = cmd(&env).args(["todo"]).output().unwrap();
    assert!(
        stdout(&out).contains("Look at Tini's PR"),
        "{}",
        stdout(&out)
    );
    assert!(
        cmd(&env)
            .args(["todo", "rm", "1"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(items(&env).len(), 1);
    assert!(
        !cmd(&env)
            .args(["todo", "rm", "1"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn sticky_round_trip_through_the_cli() {
    let env = setup();
    let out = cmd(&env)
        .args([
            "sticky",
            "set",
            "Found the 403",
            "Fixed the role hash",
            "--session",
            "app-9d",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    let out = cmd(&env)
        .args(["sticky", "show", "app-9d", "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["bullets"],
        serde_json::json!(["Found the 403", "Fixed the role hash"])
    );
    let out = cmd(&env).args(["sticky", "show"]).output().unwrap();
    assert!(!out.status.success());
}
