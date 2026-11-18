//! Stops a backup holding a file with `SECRET` in it, warns about `*.log`
//! files, and never ends for the ref `backup/loop`.

wit_bindgen::generate!({ path: "../../wit", world: "hook-plugin" });

use exports::apeiron::plugin::hook::{Change, ChangeKind, Guest, Info};

struct Hook;

impl Guest for Hook {
    fn describe() -> Info {
        Info {
            name: "test-hook".into(),
            version: "1.0".into(),
            description: "Keeps secrets out of backups".into(),
            events: vec!["backup".into()],
            permissions: vec![],
        }
    }

    fn check(
        _settings: String,
        event: String,
        ref_name: String,
        changes: Vec<Change>,
    ) -> Result<Vec<String>, String> {
        if ref_name == "backup/loop" {
            loop {
                std::hint::black_box(());
            }
        }
        let mut warnings = Vec::new();
        for c in &changes {
            if c.kind == ChangeKind::Removed {
                continue;
            }
            if c.content.as_deref().is_some_and(|d| d.windows(6).any(|w| w == b"SECRET")) {
                return Err(format!("{} holds a secret", c.path));
            }
            if c.path.ends_with(".log") {
                warnings.push(format!("{event} of {ref_name} holds {}", c.path));
            }
        }
        Ok(warnings)
    }
}

export!(Hook);
