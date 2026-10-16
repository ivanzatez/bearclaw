//! The brain: a folder of markdown notes every Claude session on this machine reads and writes.
//! Config `brain.root`, default `~/brain`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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
