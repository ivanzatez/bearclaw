//! This machine's own web server (`bearclaw web serve`), for what only it knows (memos).

use std::process::Command;

use serde_json::Value;

use crate::core::config;
use crate::core::tools::split_http_status;

/// This machine's web server: `hosts.<self>.web`, else `http://127.0.0.1:<web.port>`.
pub fn base_url() -> String {
    let cfg = config::get();
    cfg.host(&cfg.self_name())
        .and_then(|h| h.web)
        .map(|w| w.trim().trim_end_matches('/').to_string())
        .filter(|w| !w.is_empty())
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", cfg.web_port()))
}

/// GET `path` from this machine's web server.
pub fn get_json(path: &str) -> Result<Value, String> {
    get_json_at(&base_url(), path)
}

pub fn get_json_at(base: &str, path: &str) -> Result<Value, String> {
    let out = Command::new("curl")
        .args(["-sS", "--max-time", "3", "-w", "\n%{http_code}"])
        .arg(format!("{base}{path}"))
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    // 7 refused, 28 timed out, 52 empty reply, 56 reset: nobody (healthy) is listening.
    if matches!(out.status.code(), Some(7 | 28 | 52 | 56)) {
        return Err(format!(
            "web server not running at {base} — start it: bearclaw web serve (or bearclaw web install-service)"
        ));
    }
    if !out.status.success() {
        return Err(format!(
            "web server: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let (body, code) = split_http_status(&String::from_utf8_lossy(&out.stdout));
    let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if !(200..300).contains(&code) {
        let why = v["error"].as_str().map(str::to_string).unwrap_or(body);
        return Err(format!("web server {code}: {why}"));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_json_reports_a_dead_server() {
        // A port that was free a moment ago: nothing listens there.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let started = std::time::Instant::now();
        let e = get_json_at(&format!("http://127.0.0.1:{port}"), "/api/memos").unwrap_err();
        assert!(e.contains("bearclaw web serve"), "{e}");
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
    }
}
