//! Merges `*.merge` files as `base|ours|theirs`, and misbehaves when `ours`
//! asks it to: `trap`, `loop`, `alloc`, `conflict`, `refuse`, `sandbox`.

wit_bindgen::generate!({ path: "../../wit", world: "type-driver-plugin" });

use exports::apeiron::plugin::type_driver::{Guest, Info};

struct Driver;

impl Guest for Driver {
    fn describe() -> Info {
        Info {
            name: "test-driver".into(),
            version: "1.0".into(),
            description: "Misbehaves on purpose".into(),
            patterns: vec!["*.merge".into()],
            permissions: vec![],
        }
    }

    fn merge(
        base: Option<Vec<u8>>,
        ours: Vec<u8>,
        theirs: Vec<u8>,
    ) -> Result<Option<Vec<u8>>, String> {
        match ours.as_slice() {
            b"trap" => panic!("asked to crash"),
            b"loop" => loop {
                std::hint::black_box(());
            },
            b"alloc" => {
                let big = vec![1u8; 1 << 30];
                Ok(Some(big[..1].to_vec()))
            }
            b"conflict" => Ok(None),
            b"refuse" => Err("refused on purpose".into()),
            b"sandbox" => {
                let env = std::env::vars().count();
                let args = std::env::args().count();
                let files = std::fs::read_dir("/").is_ok() || std::fs::read_dir(".").is_ok();
                println!("nobody reads this");
                Ok(Some(format!("env={env} args={args} files={files}").into_bytes()))
            }
            _ => {
                let mut out = base.unwrap_or_else(|| b"-".to_vec());
                out.push(b'|');
                out.extend_from_slice(&ours);
                out.push(b'|');
                out.extend_from_slice(&theirs);
                Ok(Some(out))
            }
        }
    }

    fn diff(old: Vec<u8>, new: Vec<u8>) -> Result<Option<String>, String> {
        Ok(Some(format!("{} -> {} bytes", old.len(), new.len())))
    }
}

export!(Driver);
