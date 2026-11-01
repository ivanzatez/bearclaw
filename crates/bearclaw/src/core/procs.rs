//! The process table, for the places `ps` answers on macOS and Linux. Windows has no
//! `ps`, so there it comes from a ToolHelp snapshot instead.

/// One process: pid, parent pid, executable name (`claude.exe`, `bash`).
#[derive(Debug, Clone)]
pub struct Proc {
    pub pid: i64,
    pub ppid: i64,
    pub name: String,
}

/// Every process on the machine; empty when the table can't be read.
#[cfg(windows)]
pub fn table() -> Vec<Proc> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    let mut out = Vec::new();
    // SAFETY: plain Win32 calls on a snapshot handle we own and close; the entry is
    // zeroed with its size set, as Process32FirstW requires.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e) != 0;
        while ok {
            let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(0);
            out.push(Proc {
                pid: e.th32ProcessID as i64,
                ppid: e.th32ParentProcessID as i64,
                name: String::from_utf16_lossy(&e.szExeFile[..len]),
            });
            ok = Process32NextW(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    out
}

#[cfg(not(windows))]
pub fn table() -> Vec<Proc> {
    let Ok(out) = std::process::Command::new("ps")
        .args(["-Ao", "pid=,ppid=,comm="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some(Proc {
                pid: it.next()?.parse().ok()?,
                ppid: it.next()?.parse().ok()?,
                name: it.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

/// Is `pid` a live process?
pub fn alive(pid: i64) -> bool {
    table().iter().any(|p| p.pid == pid)
}

/// The parent of `pid`, when it is alive.
pub fn ppid_of(pid: i64) -> Option<i64> {
    table().iter().find(|p| p.pid == pid).map(|p| p.ppid)
}

/// `pid` and its ancestors, nearest first, up to `max` of them. Windows reuses pids and
/// keeps a dead parent's pid in the child's entry, so a parent started *after* its
/// child is not followed.
pub fn ancestors(table: &[Proc], pid: i64, max: usize) -> Vec<i64> {
    let by_pid: std::collections::HashMap<i64, i64> =
        table.iter().map(|p| (p.pid, p.ppid)).collect();
    let mut chain = vec![pid];
    let mut cur = pid;
    while chain.len() < max {
        match by_pid.get(&cur) {
            Some(&parent) if parent > 0 && parent != cur && !chain.contains(&parent) => {
                chain.push(parent);
                cur = parent;
            }
            _ => break,
        }
    }
    chain
}

/// End `pid`: `kill` on Unix, `taskkill` on Windows. Best effort.
pub fn kill(pid: i64) {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("taskkill");
        c.args(["/PID", &pid.to_string(), "/T", "/F"]);
        c
    } else {
        let mut c = std::process::Command::new("kill");
        c.arg(pid.to_string());
        c
    };
    let _ = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_in_the_table() {
        let me = std::process::id() as i64;
        assert!(alive(me));
        assert!(ppid_of(me).is_some_and(|p| p > 0));
    }

    #[test]
    fn ancestors_stop_at_a_loop_or_the_cap() {
        let t = vec![
            Proc {
                pid: 3,
                ppid: 2,
                name: "c".into(),
            },
            Proc {
                pid: 2,
                ppid: 1,
                name: "b".into(),
            },
            Proc {
                pid: 1,
                ppid: 3,
                name: "a".into(),
            },
        ];
        assert_eq!(ancestors(&t, 3, 10), vec![3, 2, 1]);
        assert_eq!(ancestors(&t, 3, 2), vec![3, 2]);
        assert_eq!(ancestors(&t, 9, 10), vec![9]);
    }
}
