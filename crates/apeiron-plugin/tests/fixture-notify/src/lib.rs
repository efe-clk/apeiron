//! After each backup, posts `<ref>: <changed paths>` to the address in its
//! settings.

wit_bindgen::generate!({ path: "../../wit", world: "hook-plugin" });

#[path = "../../fixture-net/http.rs"]
mod http;

use exports::apeiron::plugin::hook::{Change, Guest, Info};

struct Notify;

impl Guest for Notify {
    fn describe() -> Info {
        Info {
            name: "test-notify".into(),
            version: "1.0".into(),
            description: "Tells an address about each backup".into(),
            events: vec!["backup-done".into()],
            permissions: vec!["net:127.0.0.1".into()],
        }
    }

    fn check(
        settings: String,
        _event: String,
        ref_name: String,
        changes: Vec<Change>,
    ) -> Result<Vec<String>, String> {
        let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
        let text = format!("{ref_name}: {}", paths.join(" "));
        let (status, _) = http::request("POST", settings.trim(), text.as_bytes())?;
        if status == 200 {
            Ok(Vec::new())
        } else {
            Err(format!("the address answered {status}"))
        }
    }
}

export!(Notify);
