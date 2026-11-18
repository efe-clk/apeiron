//! Keeps repository copies on a small test server: `config` is its address.

wit_bindgen::generate!({ path: "../../wit", world: "storage-plugin" });

#[path = "../../fixture-net/http.rs"]
mod http;

use exports::apeiron::plugin::storage::{Guest, Info, SwapFailure};

struct Storage;

fn get(url: String) -> Result<Option<Vec<u8>>, String> {
    match http::request("GET", &url, &[])? {
        (200, data) => Ok(Some(data)),
        (404, _) => Ok(None),
        (status, _) => Err(format!("the server answered {status}")),
    }
}

fn send(method: &str, url: String, body: &[u8]) -> Result<(), String> {
    match http::request(method, &url, body)? {
        (200, _) => Ok(()),
        (status, _) => Err(format!("the server answered {status}")),
    }
}

fn lines(data: Vec<u8>) -> Vec<String> {
    String::from_utf8_lossy(&data)
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

impl Guest for Storage {
    fn describe() -> Info {
        Info {
            name: "test-storage".into(),
            version: "1.0".into(),
            description: "Keeps copies on the test server".into(),
            permissions: vec!["net:127.0.0.1".into()],
        }
    }

    fn read(config: String, path: String) -> Result<Option<Vec<u8>>, String> {
        get(format!("{config}/files/{path}"))
    }

    fn read_range(config: String, path: String, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let data = get(format!("{config}/files/{path}"))?.ok_or("not found")?;
        let end = (offset + len) as usize;
        data.get(offset as usize..end)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "out of range".to_string())
    }

    fn list_dir(config: String, dir: String) -> Result<Vec<String>, String> {
        Ok(get(format!("{config}/list/{dir}"))?.map(lines).unwrap_or_default())
    }

    fn write_new(config: String, path: String, data: Vec<u8>) -> Result<(), String> {
        send("PUT", format!("{config}/files/{path}"), &data)
    }

    fn create_dirs(_config: String, _dirs: Vec<String>) -> Result<(), String> {
        Ok(())
    }

    fn delete(config: String, path: String) -> Result<(), String> {
        send("DELETE", format!("{config}/files/{path}"), &[])
    }

    fn read_ref(config: String, name: String) -> Result<Option<String>, String> {
        Ok(get(format!("{config}/refs/{name}"))?.map(|d| String::from_utf8_lossy(&d).trim().to_owned()))
    }

    fn list_refs(config: String) -> Result<Vec<String>, String> {
        Ok(get(format!("{config}/refs-list"))?.map(lines).unwrap_or_default())
    }

    fn swap_ref(
        config: String,
        name: String,
        expected: Option<String>,
        new: String,
    ) -> Result<(), SwapFailure> {
        let body = format!("{}\n{new}", expected.unwrap_or_default());
        match http::request("POST", &format!("{config}/swap/{name}"), body.as_bytes()) {
            Ok((200, _)) => Ok(()),
            Ok((409, actual)) => {
                let actual = String::from_utf8_lossy(&actual).trim().to_owned();
                Err(SwapFailure::Conflict((!actual.is_empty()).then_some(actual)))
            }
            Ok((status, _)) => Err(SwapFailure::Failed(format!("the server answered {status}"))),
            Err(e) => Err(SwapFailure::Failed(e)),
        }
    }
}

export!(Storage);
