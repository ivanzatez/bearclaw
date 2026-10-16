//! The Settings tab: what bearclaw is configured to do on this machine, in one page.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use serde_json::Value;

use super::loader::Loader;
use super::pager::Pager;
use super::{Outcome, View, common_key};
use crate::core::discovery::claude_home;
use crate::core::{brain, config, productive, webapi};

pub struct SettingsView {
    config_path: PathBuf,
    claude_settings: PathBuf,
    /// Off in tests: no request to whatever listens on this machine.
    check_server: bool,
    server: Loader<()>,
    page: Pager,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new()
    }
}

fn list(v: &Value) -> String {
    match v {
        Value::Array(a) if !a.is_empty() => a
            .iter()
            .map(|x| {
                x.as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| x.to_string())
            })
            .collect::<Vec<_>>()
            .join(", "),
        Value::String(s) => s.clone(),
        _ => "—".into(),
    }
}

impl SettingsView {
    pub fn new() -> Self {
        let mut v = Self::at(config::path(), claude_home().join("settings.json"));
        v.check_server = true;
        v
    }

    pub fn at(config_path: PathBuf, claude_settings: PathBuf) -> Self {
        SettingsView {
            config_path,
            claude_settings,
            check_server: false,
            server: Loader::new(),
            page: Pager::new("Settings".into(), String::new())
                .with_hint("e edit config · r reload · j/k scroll · esc back"),
        }
    }

    fn text(&self) -> String {
        let l = config::load_from(&self.config_path);
        let c = &l.config;
        let raw = &l.raw;
        let mut t = String::new();
        let mut line = |s: String| {
            t.push_str(&s);
            t.push('\n');
        };
        line("Config".into());
        line(format!(
            "  file      {}{}",
            l.path.display(),
            if l.exists {
                ""
            } else {
                "  (missing — bearclaw init)"
            }
        ));
        if let Some(p) = &l.problem {
            line(format!("  problem   {p}"));
        }
        line(format!(
            "  host      {} (default for tmux: {})",
            c.self_name(),
            c.default_host()
        ));
        line(String::new());
        line("Web server".into());
        line(format!(
            "  dir       {}",
            config::web_dir(c).map_or("—".into(), |d| d.display().to_string())
        ));
        line(format!(
            "  listen    {}:{}",
            c.web.bind.as_deref().unwrap_or("default"),
            c.web_port()
        ));
        if self.check_server {
            let status = match (&self.server.data, &self.server.error, self.server.loading) {
                (_, _, true) => "checking…".to_string(),
                (_, Some(e), _) => e.clone(),
                (Some(()), None, _) => "up".to_string(),
                _ => "—".to_string(),
            };
            line(format!("  status    {status}"));
        }
        line(String::new());
        let root = match raw["brain"]["root"].as_str() {
            Some(r) => PathBuf::from(crate::core::tools::expand_tilde(r)),
            None => brain::root(),
        };
        let hook = std::fs::read_to_string(&self.claude_settings)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .is_some_and(|v| brain::hook_installed(&v));
        line("Brain".into());
        line(format!(
            "  folder    {} ({} notes)",
            root.display(),
            brain::list(&root).len()
        ));
        line(format!(
            "  hook installed: {}",
            if hook {
                "yes"
            } else {
                "no — bearclaw brain install"
            }
        ));
        line(String::new());
        line("Productive".into());
        match productive::Settings::from_raw(raw) {
            Ok(p) => {
                let token = std::env::var(&p.token_env)
                    .ok()
                    .filter(|t| !t.trim().is_empty());
                line(format!(
                    "  org {} · token: {} (${})",
                    p.org_id,
                    if token.is_some() {
                        "present"
                    } else {
                        "missing"
                    },
                    p.token_env
                ));
            }
            Err(e) => line(format!("  {e}")),
        }
        line(String::new());
        line("Spawn dirs".into());
        let dirs = c.spawn_dirs_for(&c.self_name());
        if dirs.is_empty() {
            line("  —".into());
        }
        for (label, path) in dirs {
            line(format!("  {label:<12} {path}"));
        }
        line(String::new());
        line("Repos".into());
        line(format!("  roots     {}", list(&raw["repos"]["roots"])));
        line(format!("  every     {}", list(&raw["repos"]["every"])));
        line(String::new());
        line("Pull requests".into());
        line(format!("  orgs      {}", list(&raw["prs"]["orgs"])));
        line(format!("  repos     {}", list(&raw["prs"]["repos"])));
        t
    }

    fn reload(&mut self) {
        if self.check_server {
            self.server
                .start(|| webapi::get_json("/api/health").map(|_| ()));
        }
        let text = self.text();
        self.page.set_text(text);
    }
}

impl View for SettingsView {
    fn on_show(&mut self) {
        self.reload();
    }

    fn tick(&mut self) {
        if self.server.poll() {
            let text = self.text();
            self.page.set_text(text);
        }
    }

    fn draw(&mut self, f: &mut Frame, area: Rect) {
        self.page.draw(f, area);
    }

    fn key(&mut self, k: KeyEvent) -> Outcome {
        if let Some(o) = common_key(k) {
            return o;
        }
        match k.code {
            KeyCode::Char('e') => return Outcome::Edit(self.config_path.clone()),
            KeyCode::Char('r') => self.reload(),
            _ => {
                self.page.key(k);
            }
        }
        Outcome::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn settings_summarise_the_config() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("config.json");
        std::fs::write(
            &cfg,
            serde_json::json!({
                "self": "laptop",
                "spawnDirs": [{"label": "Work", "paths": {"laptop": "/srv/work"}}],
                "brain": {"root": d.path().join("notes")},
                "productive": {"organizationId": "42", "tokenEnv": "BEARCLAW_TEST_NO_SUCH_TOKEN"}
            })
            .to_string(),
        )
        .unwrap();
        let claude = d.path().join("settings.json");
        std::fs::write(&claude, "{}").unwrap();
        let mut v = SettingsView::at(cfg.clone(), claude);
        v.on_show();
        let mut t = Terminal::new(TestBackend::new(110, 40)).unwrap();
        t.draw(|f| v.draw(f, f.area())).unwrap();
        let buf = t.backend().buffer().clone();
        let s: String = (0..40)
            .map(|y| (0..110).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
            .collect();
        assert!(s.contains(&cfg.display().to_string()), "{s}");
        assert!(s.contains("Work") && s.contains("/srv/work"), "{s}");
        assert!(
            s.contains("notes") && s.contains("hook installed: no"),
            "{s}"
        );
        assert!(s.contains("org 42") && s.contains("token: missing"), "{s}");
        assert_eq!(
            v.key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char('e'),
                crossterm::event::KeyModifiers::NONE
            )),
            Outcome::Edit(cfg)
        );
    }
}
