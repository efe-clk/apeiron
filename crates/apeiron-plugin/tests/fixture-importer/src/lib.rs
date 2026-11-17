//! Reads `*.lines` archives: each line is `path=content`. A line `trap`
//! crashes it and a line `refuse` makes it refuse.

wit_bindgen::generate!({ path: "../../wit", world: "importer-plugin" });

use exports::apeiron::plugin::importer::{File, Guest, Info};

struct Importer;

impl Guest for Importer {
    fn describe() -> Info {
        Info {
            name: "test-importer".into(),
            version: "1.0".into(),
            description: "Reads path=content lines".into(),
            patterns: vec!["*.lines".into()],
            permissions: vec![],
        }
    }

    fn read(content: Vec<u8>) -> Result<Vec<File>, String> {
        let text = String::from_utf8(content).map_err(|_| "not UTF-8".to_string())?;
        let mut files = Vec::new();
        for line in text.lines() {
            match line {
                "trap" => panic!("asked to crash"),
                "refuse" => return Err("refused on purpose".into()),
                _ => {
                    let (path, content) = line.split_once('=').ok_or("a line without =")?;
                    files.push(File {
                        path: path.into(),
                        content: content.as_bytes().to_vec(),
                    });
                }
            }
        }
        Ok(files)
    }
}

export!(Importer);
