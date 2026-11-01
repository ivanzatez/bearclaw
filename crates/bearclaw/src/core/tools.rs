//! Small environment helpers: locating binaries, expanding `~`, shell quoting.
//!
//! A `bearclaw` started over ssh runs under a non-login shell whose PATH often lacks
//! Homebrew and `~/.local/bin`, so every external tool is resolved here, with the
//! usual install locations as fallbacks, instead of trusting a bare name.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The home directory: `$HOME`, as on Unix (where `dirs` reads it too). On Windows `dirs`
/// asks the shell for the profile folder and ignores `$HOME`, so the tests' temp homes
/// would land in the real one; there `$HOME` counts when it is an absolute Windows path
/// (Git Bash's `/c/Users/x` is not).
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
        .or_else(dirs::home_dir)
}

/// Is `p` absolute: `/x` (what a Unix host, this one or a remote one, means) or, on
/// Windows, `C:\x` / `C:/x`.
pub fn is_abs(p: &str) -> bool {
    p.starts_with('/') || Path::new(p).is_absolute()
}

/// A path separator: `/`, and on Windows `\` too.
pub fn is_sep(c: char) -> bool {
    c == '/' || (cfg!(windows) && c == '\\')
}

/// Directories searched after `$PATH`.
fn fallback_dirs() -> Vec<PathBuf> {
    let mut dirs = if cfg!(windows) {
        // Git for Windows carries the POSIX tools (`sh`, `cat`, `env`) every shell line
        // bearclaw writes relies on; winget installs land in its Links dir.
        let pf = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        let mut d = vec![
            pf.join(r"Git\cmd"),
            pf.join(r"Git\bin"),
            pf.join(r"Git\usr\bin"),
            pf.join(r"nodejs"),
        ];
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            d.push(local.join(r"Microsoft\WinGet\Links"));
        }
        d
    } else {
        vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]
    };
    if let Some(home) = home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join("bin"));
    }
    dirs
}

/// `$PATH` with the fallback dirs appended — for a child that itself looks things up
/// (pi's launcher is a script that runs `node`).
pub fn search_path() -> std::ffi::OsString {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::env::split_paths(&path).chain(fallback_dirs())).unwrap_or(path)
}

#[cfg(unix)]
pub fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Windows has no execute bit: a file with one of the `PATHEXT` extensions runs.
#[cfg(windows)]
pub fn is_executable(p: &Path) -> bool {
    let runs = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["exe", "cmd", "bat", "com"].contains(&e.to_ascii_lowercase().as_str()));
    runs && p.is_file()
}

/// The names `name` can have on disk: itself, and on Windows with each runnable extension.
fn candidates(name: &str) -> Vec<String> {
    if cfg!(windows) && Path::new(name).extension().is_none() {
        ["exe", "cmd", "bat", "com"]
            .iter()
            .map(|e| format!("{name}.{e}"))
            .collect()
    } else {
        vec![name.to_string()]
    }
}

/// First executable called `name` on `$PATH`, then in the fallback dirs.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let names = candidates(name);
    std::env::split_paths(&path)
        .chain(fallback_dirs())
        .flat_map(|d| names.iter().map(move |n| d.join(n)).collect::<Vec<_>>())
        .find(|p| is_executable(p))
}

/// The POSIX shell for `sh -c` lines: `sh` itself, or Git for Windows' `sh.exe`.
pub fn sh() -> &'static str {
    static SH: OnceLock<String> = OnceLock::new();
    SH.get_or_init(|| {
        if cfg!(windows) {
            git_bash().unwrap_or_else(|| resolve_binary("sh", None))
        } else {
            "sh".to_string()
        }
    })
}

/// Git for Windows' `bash.exe` (not WSL's `bash.exe` in System32): the shell tmux panes
/// run on Windows, so the POSIX lines bearclaw types into them work as they do on a Mac.
pub fn git_bash() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from)?;
    [
        pf.join(r"Git\bin\bash.exe"),
        pf.join(r"Git\usr\bin\bash.exe"),
    ]
    .into_iter()
    .chain(find_binary("git").and_then(|g| {
        // <root>\cmd\git.exe -> <root>\bin\bash.exe
        Some(g.parent()?.parent()?.join(r"bin\bash.exe"))
    }))
    .find(|p| p.is_file())
    .map(|p| p.display().to_string())
}

/// `explicit` (a configured path, `~` allowed) when set, else [`find_binary`],
/// else the bare name so the spawn reports a clear "not found".
pub fn resolve_binary(name: &str, explicit: Option<&str>) -> String {
    if let Some(e) = explicit.map(str::trim).filter(|e| !e.is_empty()) {
        return expand_tilde(e);
    }
    find_binary(name)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| name.to_string())
}

/// The tmux binary: `$BEARCLAW_TMUX`, then config `tmux`, then PATH + fallbacks.
/// Resolved once per process.
pub fn tmux() -> &'static str {
    static TMUX: OnceLock<String> = OnceLock::new();
    TMUX.get_or_init(|| {
        let env = std::env::var("BEARCLAW_TMUX").ok();
        let cfg = crate::core::config::load().config.tmux.clone();
        resolve_binary("tmux", env.as_deref().or(cfg.as_deref()))
    })
}

/// A `tmux` command with `-u`: without a UTF-8 locale (launchd, cron, `env -i`) tmux
/// escapes non-ASCII and control characters in its output — `-F` tab separators come
/// back as `_`, so pane discovery found nothing — and mangles UTF-8 typed via send-keys.
pub fn tmux_cmd() -> std::process::Command {
    let mut cmd = std::process::Command::new(tmux());
    cmd.arg("-u");
    cmd
}

/// The `claude` binary used for headless naming calls.
pub fn claude() -> Option<String> {
    find_binary("claude")
        .or_else(|| {
            home_dir()
                .map(|h| h.join(".claude/local/claude"))
                .filter(|p| is_executable(p))
        })
        .map(|p| p.display().to_string())
}

/// Expand a leading `~` / `~/` against `$HOME`. Anything else passes through.
pub fn expand_tilde(p: &str) -> String {
    let Some(home) = home_dir() else {
        return p.to_string();
    };
    if p == "~" {
        return home.display().to_string();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return home.join(rest).display().to_string();
    }
    p.to_string()
}

/// Rewrite an absolute path under this machine's `$HOME` to `~/…`, so it means
/// "the same place under *your* home" on a remote host.
pub fn tildify(p: &str) -> String {
    match home_dir().and_then(|h| h.to_str().map(String::from)) {
        Some(home) if p == home => "~".into(),
        Some(home)
            if p.strip_prefix(home.as_str())
                .is_some_and(|r| r.starts_with(is_sep)) =>
        {
            format!("~{}", &p[home.len()..])
        }
        _ => p.to_string(),
    }
}

/// Single-quote a value for a POSIX shell.
pub fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Quote an argument only when it needs it — keeps dry-run output readable.
/// A leading `=` is quoted too: zsh expands `=word` to a command path.
pub fn shq_min(s: &str) -> String {
    if !s.is_empty()
        && !s.starts_with('=')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/@%+,:=-".contains(c))
    {
        s.to_string()
    } else {
        shq(s)
    }
}

/// A path for a remote shell: `~` / `~/x` becomes `"$HOME"/x` so it expands over
/// there; anything else is quoted.
pub fn remote_path(p: &str) -> String {
    if p == "~" {
        return "\"$HOME\"".into();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        return format!("\"$HOME\"/{}", shq_min(rest));
    }
    shq(p)
}

/// Unix permission bits for a file or directory being created (`0o600`, `0o700`).
/// Windows has no mode bits — a profile directory is private to its user already — so
/// there the mode is ignored.
pub trait PrivateMode {
    fn private(&mut self, mode: u32) -> &mut Self;
}

impl PrivateMode for std::fs::OpenOptions {
    fn private(&mut self, mode: u32) -> &mut Self {
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(self, mode);
        let _ = mode;
        self
    }
}

impl PrivateMode for std::fs::DirBuilder {
    fn private(&mut self, mode: u32) -> &mut Self {
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(self, mode);
        let _ = mode;
        self
    }
}

/// Truthy env flag: `1`, `true`, `yes`.
pub fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|v| matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

/// Is stdin *and* stdout a terminal?
pub fn interactive() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Short hostname, lowercased (`hostname -s`), used only as a default name.
pub fn short_hostname() -> String {
    if cfg!(windows) {
        return std::env::var("COMPUTERNAME")
            .map(|h| h.trim().to_lowercase())
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "local".into());
    }
    std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_lowercase())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "local".into())
}

/// Lowercase, runs of anything but letters and digits → one `-`, no `-` at the ends.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Split `curl -w "\n%{http_code}"` output into the body and the status (0 when missing).
pub fn split_http_status(out: &str) -> (String, u16) {
    match out.rsplit_once('\n') {
        Some((body, code)) => (body.to_string(), code.trim().parse().unwrap_or(0)),
        None => (out.to_string(), 0),
    }
}

/// A launcher with a shell-wide `ANTHROPIC_MODEL` dropped, so a session bearclaw starts runs on
/// Claude Code's own default model, whatever a `.zshrc` pins for terminal sessions.
pub fn without_model_env(launcher: &str) -> String {
    format!("env -u ANTHROPIC_MODEL {launcher}")
}

/// Open an `http(s)` link in the default browser (`open`, else `xdg-open`). Anything else is
/// refused: these links come from APIs, and `open` would happily run a file.
pub fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(format!("not a web link: {url:?}"));
    }
    let bin = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        // The URL handler, not `cmd /c start`: cmd would read `&` in a query string.
        "rundll32"
    } else {
        "xdg-open"
    };
    let mut cmd = std::process::Command::new(bin);
    if cfg!(windows) {
        cmd.arg("url.dll,FileProtocolHandler");
    }
    let mut child = cmd
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{bin}: {e}"))?;
    // Reap it off the caller's thread; the dashboard must not wait on a browser.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_and_http_status() {
        assert_eq!(slugify("Acme Ltd."), "acme-ltd");
        assert_eq!(slugify("  Hello,  World!! "), "hello-world");
        assert_eq!(slugify("Čvrsti 2 koraka"), "čvrsti-2-koraka");
        assert_eq!(slugify("!!!"), "");
        assert_eq!(
            split_http_status("{\"a\":1}\n200"),
            ("{\"a\":1}".to_string(), 200)
        );
        assert_eq!(split_http_status("\n000"), (String::new(), 0));
        assert_eq!(split_http_status("garbage"), ("garbage".to_string(), 0));
    }

    #[test]
    fn default_model_drops_a_shell_wide_pin() {
        // A session the user opens in bearclaw runs on Claude Code's default, not a .zshrc pin.
        assert_eq!(
            without_model_env("claude --dangerously-skip-permissions"),
            "env -u ANTHROPIC_MODEL claude --dangerously-skip-permissions"
        );
    }

    #[test]
    fn open_url_rejects_non_http() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
        assert!(open_url("").is_err());
    }

    #[test]
    fn quoting() {
        assert_eq!(shq("a b"), "'a b'");
        assert_eq!(shq("it's"), r"'it'\''s'");
        assert_eq!(shq_min("plain-word_1"), "plain-word_1");
        assert_eq!(shq_min("two words"), "'two words'");
        assert_eq!(shq_min(""), "''");
        assert_eq!(shq_min("=session:"), "'=session:'");
        assert_eq!(shq_min("a=b"), "a=b");
    }

    #[test]
    fn remote_paths_expand_over_there() {
        assert_eq!(remote_path("~"), "\"$HOME\"");
        assert_eq!(remote_path("~/Code/x"), "\"$HOME\"/Code/x");
        assert_eq!(remote_path("~/a b"), "\"$HOME\"/'a b'");
        assert_eq!(remote_path("/tmp/x"), "'/tmp/x'");
    }

    #[test]
    fn tilde_round_trip() {
        let home = home_dir().unwrap().display().to_string();
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(
            expand_tilde("~/x"),
            std::path::Path::new(&home).join("x").display().to_string()
        );
        assert_eq!(expand_tilde("/abs"), "/abs");
        assert_eq!(tildify(&format!("{home}/x/y")), "~/x/y");
        assert_eq!(tildify(&home), "~");
        assert_eq!(tildify("/elsewhere"), "/elsewhere");
        // A sibling directory that merely shares the prefix is not under $HOME.
        assert_eq!(tildify(&format!("{home}x")), format!("{home}x"));
    }
}
