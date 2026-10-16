//! `bearclaw tmux …` against a private tmux server (TMUX_TMPDIR in a temp dir), so
//! the user's own sessions are never visible, let alone touched.

mod common;

use common::{Env, stderr, stdout};

struct Server {
    env: Env,
}

impl Server {
    /// `None` when tmux isn't installed — the tests then pass vacuously.
    fn new() -> Option<Self> {
        bearclaw::core::tools::find_binary("tmux")?;
        let env = Env::new();
        std::fs::create_dir_all(env.path("tmux")).unwrap();
        std::fs::create_dir_all(env.path("home")).unwrap();
        Some(Server { env })
    }

    fn cmd(&self) -> assert_cmd::Command {
        let mut c = self.env.cmd();
        c.env("TMUX_TMPDIR", self.env.path("tmux"))
            .env("HOME", self.env.path("home"))
            .env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .timeout(std::time::Duration::from_secs(20));
        c
    }

    fn bearclaw(&self, args: &[&str]) -> std::process::Output {
        self.cmd().args(args).output().unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::process::Command::new(bearclaw::core::tools::find_binary("tmux").unwrap())
            .arg("kill-server")
            .env("TMUX_TMPDIR", self.env.path("tmux"))
            .env_remove("TMUX")
            .output();
    }
}

#[test]
fn sessions_lifecycle() {
    let Some(s) = Server::new() else { return };

    let out = s.bearclaw(&["tmux", "list"]);
    assert!(out.status.success());
    assert!(
        stderr(&out).contains("no tmux sessions"),
        "{}",
        stderr(&out)
    );

    let out = s.bearclaw(&["new", "bearclaw test alpha!", "-d"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("created (detached): bearclaw-test-alpha"),
        "{}",
        stdout(&out)
    );
    let out = s.bearclaw(&[
        "tmux",
        "new",
        "-d",
        "bearclaw-test-beta",
        "--",
        "sleep",
        "300",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    // Creating it again is a note, not a failure.
    let out = s.bearclaw(&["tmux", "new", "-d", "bearclaw-test-beta"]);
    assert!(out.status.success());
    assert!(stderr(&out).contains("already exists"));

    let out = s.bearclaw(&["tmux", "ls", "-q"]);
    let names = stdout(&out);
    assert!(names.lines().any(|l| l == "bearclaw-test-alpha"), "{names}");
    assert!(names.lines().any(|l| l == "bearclaw-test-beta"), "{names}");

    let out = s.bearclaw(&["t", "list", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let alpha = v
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "bearclaw-test-alpha")
        .unwrap();
    assert_eq!(alpha["attached"], 0);
    assert_eq!(alpha["windows"], 1);
    assert_eq!(alpha["current"], false);
    assert_eq!(alpha["host"], "local");

    // Ambiguous → 2, no match → 3, no terminal → 3.
    let out = s.bearclaw(&["tmux", "kill", "bearclaw-test", "-f"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert!(stderr(&out).contains("matches 2 sessions"));
    let out = s.bearclaw(&["tmux", "kill", "zzz-nothing", "-f"]);
    assert_eq!(out.status.code(), Some(3));
    let out = s.bearclaw(&["enter", "alpha"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(
        stderr(&out).contains("needs a terminal"),
        "{}",
        stderr(&out)
    );
    // Killing without -f needs a terminal to confirm on — refused, not assumed.
    let out = s.bearclaw(&["tmux", "kill", "alpha"]);
    assert!(!out.status.success());

    // Rename: collision refused, then a real one.
    let out = s.bearclaw(&["tmux", "rename", "alpha", "bearclaw-test-beta"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("already exists"), "{}", stderr(&out));
    let out = s.bearclaw(&["tmux", "rename", "alpha", "bearclaw test gamma"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("bearclaw-test-alpha → bearclaw-test-gamma"));

    // Dry-run prints the tmux command and leaves the session alone.
    let out = s.bearclaw(&["-n", "tmux", "kill", "gamma", "-f"]);
    assert!(
        stdout(&out).contains("kill-session -t '=bearclaw-test-gamma'"),
        "{}",
        stdout(&out)
    );

    let out = s.bearclaw(&["tmux", "kill", "gamma", "-f"]);
    assert!(out.status.success());
    assert_eq!(stdout(&out).trim(), "killed: bearclaw-test-gamma");
    let out = s.bearclaw(&["tmux", "list", "-q"]);
    assert_eq!(stdout(&out).trim(), "bearclaw-test-beta");
}

#[test]
fn stale_lists_idle_shells_and_keeps_busy_ones() {
    let Some(s) = Server::new() else { return };
    assert!(
        s.bearclaw(&["new", "-d", "bearclaw-test-idle"])
            .status
            .success()
    );
    assert!(
        s.bearclaw(&["new", "-d", "bearclaw-test-busy", "--", "sleep", "300"])
            .status
            .success()
    );
    // Give the shell a moment to start.
    std::thread::sleep(std::time::Duration::from_millis(500));

    let out = s.bearclaw(&["tmux", "stale", "--older-than", "0", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["claude_checked"], true,
        "an empty registry is a clean check: {v}"
    );
    let cands: Vec<&str> = v["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(cands, ["bearclaw-test-idle"], "{v}");
    let busy = v["kept"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["name"] == "bearclaw-test-busy")
        .unwrap();
    assert_eq!(busy["class"], "running");

    let out = s.bearclaw(&["tmux", "stale"]);
    assert!(
        stdout(&out).contains("no stale sessions"),
        "24h threshold: {}",
        stdout(&out)
    );
    let out = s.bearclaw(&["tmux", "stale", "--older-than", "0", "-q"]);
    assert_eq!(stdout(&out).trim(), "bearclaw-test-idle");

    // --no-bearclaw-check with -f is refused outright.
    let out = s.bearclaw(&["tmux", "stale", "--kill", "-f", "--no-bearclaw-check"]);
    assert_eq!(out.status.code(), Some(1));
    let out = s.bearclaw(&["tmux", "stale", "--older-than", "5w"]);
    assert_eq!(out.status.code(), Some(1));

    let out = s.bearclaw(&["tmux", "stale", "--older-than", "0", "--kill", "-f"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stdout(&out).contains("killed: bearclaw-test-idle"),
        "{}",
        stdout(&out)
    );
    let out = s.bearclaw(&["tmux", "list", "-q"]);
    assert_eq!(stdout(&out).trim(), "bearclaw-test-busy");
}

#[test]
fn enter_falls_back_to_other_hosts_when_the_default_has_no_match() {
    let Some(s) = Server::new() else { return };
    // defaultHost = workstation, which cannot be reached; this machine = laptop.
    let mut cfg = common::two_hosts();
    cfg["hosts"]["workstation"]["ssh"] = serde_json::json!("bearclaw-test-host.invalid");
    s.env.write_config(&cfg);
    let out = s.bearclaw(&["--local", "tmux", "new", "-d", "bearclaw-test-local"]);
    assert!(out.status.success(), "{}", stderr(&out));

    // Found here → a note, then the (dry-run) attach on this machine.
    let out = s.bearclaw(&["-n", "enter", "test-loc"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains("workstation:"),
        "unreachable host warned: {err}"
    );
    assert!(err.contains("→ laptop: bearclaw-test-local"), "{err}");
    assert!(
        stdout(&out).contains("attach -t '=bearclaw-test-local'"),
        "{}",
        stdout(&out)
    );

    // Nothing anywhere → 3, naming the hosts that answered.
    let out = s.bearclaw(&["-n", "enter", "zzz-nothing"]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("no session matching 'zzz-nothing' on laptop"),
        "{}",
        stderr(&out)
    );

    // A named host means no fallback: straight to that host.
    let out = s.bearclaw(&["-n", "--local", "enter", "zzz-nothing"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(!stderr(&out).contains("workstation"), "{}", stderr(&out));
}
