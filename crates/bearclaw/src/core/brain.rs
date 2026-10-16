//! The brain: a folder of markdown notes every Claude session on this machine reads and writes.
//! Config `brain.root`, default `~/brain`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::core::config;
use crate::core::tools::{expand_tilde, slugify};

/// `brain.root` (read fresh, so a config edit applies), else `~/brain`.
pub fn root() -> PathBuf {
    let raw = config::load_from(&config::path()).raw;
    match raw["brain"]["root"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(r) => PathBuf::from(expand_tilde(r)),
        None => dirs::home_dir().unwrap_or_default().join("brain"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    pub path: PathBuf,
    /// Relative to the root, `/`-separated.
    pub rel: String,
    pub title: String,
    pub mtime_ms: i64,
}

/// The first `# ` heading after an optional `---` frontmatter block, else `stem`.
pub fn title_of(text: &str, stem: &str) -> String {
    let mut lines = text.lines().peekable();
    if lines.peek().map(|l| l.trim()) == Some("---") {
        lines.next();
        for l in lines.by_ref() {
            if l.trim() == "---" {
                break;
            }
        }
    }
    lines
        .find_map(|l| l.strip_prefix("# ").map(str::trim))
        .filter(|t| !t.is_empty())
        .unwrap_or(stem)
        .to_string()
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Note>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let path = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            walk(root, &path, out);
            continue;
        }
        if !name.ends_with(".md") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).unwrap_or_default();
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as i64);
        out.push(Note {
            title: title_of(&text, name.trim_end_matches(".md")),
            path,
            rel,
            mtime_ms,
        });
    }
}

/// Every `*.md` under `root` (dot-dirs and `node_modules` skipped), newest first. A missing
/// root is an empty brain.
pub fn list(root: &Path) -> Vec<Note> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then_with(|| a.rel.cmp(&b.rel)));
    out
}

/// Indices of the notes whose title or text holds every whitespace-separated term of `q`
/// (case-insensitive). An empty query matches everything.
pub fn search(notes: &[Note], q: &str) -> Vec<usize> {
    let terms: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return (0..notes.len()).collect();
    }
    (0..notes.len())
        .filter(|&i| {
            let n = &notes[i];
            let hay = format!(
                "{}\n{}",
                n.title,
                fs::read_to_string(&n.path).unwrap_or_default()
            )
            .to_lowercase();
            terms.iter().all(|t| hay.contains(t.as_str()))
        })
        .collect()
}

/// The hook's whole output is capped near this; Claude Code truncates long hook context.
const CONTEXT_LIMIT: usize = 8000;

/// What every Claude session is told at start: where the brain is, how to use it, and the
/// newest `cap` note titles.
pub fn hook_context(root: &Path, notes: &[Note], cap: usize) -> String {
    let root = root.display();
    let mut out = format!(
        "# Brain — shared notes for every Claude session on this machine\n\
The brain is a folder of markdown notes at {root}. Every Claude Code session here reads and writes it.\n\
- Read the notes below whose titles look relevant before you start (Read tool); Grep the folder for topics not listed.\n\
- When you learn something durable and reusable — how a system works, a gotcha, a decision and its reason, a preference of the user — write it down: update the note that covers the topic, or create {root}/<topic-slug>.md starting with \"# Title\". One topic per note. Task progress, temporary state and secrets do not belong here.\n\
- When the user asks you to remember something, save it to the brain, or to check the brain, this is the place.\n\
Notes (newest first):\n"
    );
    if notes.is_empty() {
        out.push_str("(no notes yet)\n");
        return out;
    }
    let mut listed = 0;
    for n in notes.iter().take(cap) {
        let line = format!("- {} — {}\n", n.title, n.rel);
        // Room for the "+N more" line after it.
        if out.len() + line.len() + 64 > CONTEXT_LIMIT {
            break;
        }
        out.push_str(&line);
        listed += 1;
    }
    if listed < notes.len() {
        out.push_str(&format!(
            "(+{} more — search the folder)\n",
            notes.len() - listed
        ));
    }
    out
}

/// Ours is any SessionStart hook whose command ends this way, whatever path runs it.
const HOOK_SUFFIX: &str = "brain hook";

fn is_ours(h: &Value) -> bool {
    h["command"]
        .as_str()
        .is_some_and(|c| c.trim_end().ends_with(HOOK_SUFFIX))
}

/// Put our SessionStart hook in a parsed `settings.json`: replace the command of an existing
/// one in place, else append an entry. Everything else stays as it was. True when it changed.
pub fn install_hook(settings: &mut Value, command: &str) -> bool {
    if !settings.is_object() {
        *settings = Value::Object(Default::default());
    }
    for entry in settings["hooks"]["SessionStart"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        for h in entry["hooks"].as_array_mut().into_iter().flatten() {
            if is_ours(h) {
                if h["command"] == command {
                    return false;
                }
                h["command"] = Value::from(command);
                return true;
            }
        }
    }
    if !settings["hooks"].is_object() {
        settings["hooks"] = Value::Object(Default::default());
    }
    if !settings["hooks"]["SessionStart"].is_array() {
        settings["hooks"]["SessionStart"] = Value::Array(Vec::new());
    }
    if let Some(list) = settings["hooks"]["SessionStart"].as_array_mut() {
        list.push(serde_json::json!({"hooks": [{"type": "command", "command": command}]}));
    }
    true
}

/// Take our hook back out, and the `SessionStart` / `hooks` containers it leaves empty.
pub fn uninstall_hook(settings: &mut Value) -> bool {
    let Some(list) = settings["hooks"]["SessionStart"].as_array_mut() else {
        return false;
    };
    let mut changed = false;
    for entry in list.iter_mut() {
        if let Some(hooks) = entry["hooks"].as_array_mut() {
            let before = hooks.len();
            hooks.retain(|h| !is_ours(h));
            changed |= hooks.len() != before;
        }
    }
    list.retain(|e| e["hooks"].as_array().is_none_or(|h| !h.is_empty()));
    if list.is_empty()
        && let Some(hooks) = settings["hooks"].as_object_mut()
    {
        hooks.remove("SessionStart");
        if hooks.is_empty()
            && let Some(top) = settings.as_object_mut()
        {
            top.remove("hooks");
        }
    }
    changed
}

/// A new note `<slug>.md` starting with `# title`; `-2`, `-3`… rather than overwrite anything.
pub fn create(root: &Path, title: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(root)?;
    let mut slug = slugify(title);
    if slug.is_empty() {
        slug = "note".into();
    }
    for n in 1.. {
        let name = if n == 1 {
            format!("{slug}.md")
        } else {
            format!("{slug}-{n}.md")
        };
        let path = root.join(name);
        match fs::File::options().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(format!("# {}\n\n", title.trim()).as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("1.. never ends")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn write(root: &Path, rel: &str, text: &str, age_secs: u64) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, text).unwrap();
        let f = fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(age_secs))
            .unwrap();
    }

    #[test]
    fn titles_come_from_the_first_heading() {
        assert_eq!(
            title_of("---\ntags: x\n---\n# Deploys\nbody", "deploys"),
            "Deploys"
        );
        assert_eq!(title_of("intro\n\n## Sub\n# Main", "x"), "Main");
        assert_eq!(title_of("no heading", "stem"), "stem");
    }

    #[test]
    fn list_skips_hidden_and_survives_a_missing_root() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "a.md", "# A", 100);
        write(d.path(), "sub/b.md", "# B", 10);
        write(d.path(), ".git/x.md", "# X", 1);
        write(d.path(), "node_modules/y.md", "# Y", 1);
        write(d.path(), "c.txt", "# C", 1);
        let notes = list(d.path());
        let rels: Vec<&str> = notes.iter().map(|n| n.rel.as_str()).collect();
        assert_eq!(rels, ["sub/b.md", "a.md"]);
        assert_eq!(notes[0].title, "B");
        assert!(list(&d.path().join("missing")).is_empty());
    }

    #[test]
    fn search_needs_every_term() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "api.md", "# Deploy api\nuses helm", 2);
        write(d.path(), "web.md", "# Deploy web\nuses vercel", 1);
        let notes = list(d.path());
        let hits: Vec<&str> = search(&notes, "deploy HELM")
            .into_iter()
            .map(|i| notes[i].rel.as_str())
            .collect();
        assert_eq!(hits, ["api.md"]);
        assert_eq!(search(&notes, "  ").len(), 2);
    }

    fn note(title: &str, rel: &str) -> Note {
        Note {
            path: PathBuf::from(rel),
            rel: rel.into(),
            title: title.into(),
            mtime_ms: 0,
        }
    }

    #[test]
    fn hook_context_lists_and_caps() {
        let root = Path::new("/notes");
        let notes = [
            note("One", "one.md"),
            note("Two", "a/two.md"),
            note("Three", "three.md"),
        ];
        let c = hook_context(root, &notes, 2);
        assert!(c.contains("/notes"), "{c}");
        assert!(
            c.contains("- One — one.md") && c.contains("- Two — a/two.md"),
            "{c}"
        );
        assert!(!c.contains("Three") && c.contains("(+1 more"), "{c}");
        assert!(hook_context(root, &[], 100).contains("(no notes yet)"));
        let many: Vec<Note> = (0..2000)
            .map(|i| note(&"x".repeat(40), &format!("{i}.md")))
            .collect();
        assert!(hook_context(root, &many, 2000).len() <= 8000);
    }

    #[test]
    fn install_is_idempotent_and_preserving() {
        let original = serde_json::json!({
            "model": "x",
            "hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "other"}]}], "Stop": []}
        });
        let mut v = original.clone();
        assert!(install_hook(&mut v, "/bin/bearclaw brain hook"));
        let starts = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[0]["hooks"][0]["command"], "other");
        assert_eq!(starts[1]["hooks"][0]["command"], "/bin/bearclaw brain hook");
        let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "hooks"]);
        assert!(v["hooks"]["Stop"].is_array());

        assert!(!install_hook(&mut v, "/bin/bearclaw brain hook"));
        assert!(install_hook(&mut v, "/new/bearclaw brain hook"));
        let starts = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(starts.len(), 2);
        assert_eq!(starts[1]["hooks"][0]["command"], "/new/bearclaw brain hook");

        assert!(uninstall_hook(&mut v));
        assert_eq!(v, original);
        assert!(!uninstall_hook(&mut v));

        let mut empty = serde_json::json!({});
        assert!(install_hook(&mut empty, "b brain hook"));
        assert!(uninstall_hook(&mut empty));
        assert_eq!(empty, serde_json::json!({}));
    }

    #[test]
    fn create_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("brain");
        let a = create(&root, "Deploys").unwrap();
        fs::write(&a, "# Deploys\nkeep me").unwrap();
        let b = create(&root, "Deploys").unwrap();
        assert_eq!(a.file_name().unwrap(), "deploys.md");
        assert_eq!(b.file_name().unwrap(), "deploys-2.md");
        assert_eq!(fs::read_to_string(&a).unwrap(), "# Deploys\nkeep me");
        assert_eq!(fs::read_to_string(&b).unwrap(), "# Deploys\n\n");
        assert_eq!(
            create(&root, "!!!").unwrap().file_name().unwrap(),
            "note.md"
        );
    }
}
