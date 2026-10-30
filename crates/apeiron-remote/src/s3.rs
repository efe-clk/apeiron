//! Remotes in S3 and S3-compatible storage (FORMAT.md §15.2).

use std::fmt::Display;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use apeiron_core::{Error, ObjectId, Result, S3Url, Store, decode_ref, encode_ref};
use object_store::aws::{AmazonS3Builder, Checksum, S3ConditionalPut};
use object_store::path::Path;
use object_store::{
    Error as StoreError, ObjectStore, ObjectStoreExt, PutMode, PutOptions, PutPayload,
    UpdateVersion,
};
use tokio::runtime::Runtime;

/// A remote in an S3 bucket. Every call blocks until the service has
/// answered. Files are replaced only with conditional writes, so two
/// writers never overwrite each other's ref.
pub struct S3Store {
    store: Arc<dyn ObjectStore>,
    rt: Runtime,
    /// The folder inside the bucket, without `/` at either end.
    prefix: String,
    /// `s3://bucket`, for messages.
    origin: String,
}

impl std::fmt::Debug for S3Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "S3Store({}/{})", self.origin, self.prefix)
    }
}

impl S3Store {
    /// Uses the credentials in `AWS_ACCESS_KEY_ID` and
    /// `AWS_SECRET_ACCESS_KEY` (FORMAT.md §15.1).
    pub fn connect(url: &S3Url) -> Result<Self> {
        let mut builder = AmazonS3Builder::from_env()
            .with_bucket_name(&url.bucket)
            .with_conditional_put(S3ConditionalPut::ETagMatch)
            // A bucket with Object Lock takes only writes with a checksum
            // (ARCHIVE.md §4); it also guards every write on the way.
            .with_checksum_algorithm(Checksum::SHA256);
        if let Some(endpoint) = &url.endpoint {
            builder = builder
                .with_endpoint(endpoint)
                .with_allow_http(endpoint.starts_with("http://"));
        }
        if let Some(region) = &url.region {
            builder = builder.with_region(region);
        }
        let origin = format!("s3://{}", url.bucket);
        let store = builder.build().map_err(|e| Error::Io {
            path: PathBuf::from(&origin),
            source: io::Error::other(e.to_string()),
        })?;
        S3Store::new(Arc::new(store), &url.prefix, origin)
    }

    fn new(store: Arc<dyn ObjectStore>, prefix: &str, origin: String) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| Error::Io {
                path: PathBuf::from(&origin),
                source: e,
            })?;
        Ok(S3Store {
            store,
            rt,
            prefix: prefix.to_owned(),
            origin,
        })
    }

    fn key(&self, rel: &str) -> Path {
        match (self.prefix.as_str(), rel) {
            (prefix, "") => Path::from(prefix),
            ("", rel) => Path::from(rel),
            (prefix, rel) => Path::from(format!("{prefix}/{rel}")),
        }
    }

    fn error(&self, rel: &str, e: impl Display) -> Error {
        Error::Io {
            path: self.location(rel),
            source: io::Error::other(e.to_string()),
        }
    }

    /// The content and version of a file; `None` if it does not exist.
    async fn get(&self, rel: &str) -> Result<Option<(Vec<u8>, UpdateVersion)>> {
        let result = match self.store.get(&self.key(rel)).await {
            Ok(result) => result,
            Err(StoreError::NotFound { .. }) => return Ok(None),
            Err(e) => return Err(self.error(rel, e)),
        };
        let version = UpdateVersion {
            e_tag: result.meta.e_tag.clone(),
            version: result.meta.version.clone(),
        };
        let bytes = result.bytes().await.map_err(|e| self.error(rel, e))?;
        Ok(Some((bytes.to_vec(), version)))
    }

    async fn read_ref_async(&self, name: &str) -> Result<Option<(ObjectId, UpdateVersion)>> {
        apeiron_core::validate_ref_name(name)?;
        let rel = format!("refs/{name}");
        let Some((bytes, version)) = self.get(&rel).await? else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&bytes);
        Ok(Some((decode_ref(&self.location(&rel), &text)?, version)))
    }

    async fn swap_ref_async(
        &self,
        name: &str,
        expected: Option<ObjectId>,
        new: ObjectId,
    ) -> Result<()> {
        let current = self.read_ref_async(name).await?;
        let actual = current.as_ref().map(|(id, _)| *id);
        let conflict = |actual| Error::RefConflict {
            name: name.to_owned(),
            expected,
            actual,
        };
        if actual != expected {
            return Err(conflict(actual));
        }
        let mode = match current {
            Some((_, version)) => PutMode::Update(version),
            None => PutMode::Create,
        };
        let rel = format!("refs/{name}");
        let payload = PutPayload::from(encode_ref(new).into_bytes());
        match self
            .store
            .put_opts(&self.key(&rel), payload, mode.into())
            .await
        {
            Ok(_) => Ok(()),
            // Another writer changed the ref after it was read.
            Err(StoreError::AlreadyExists { .. } | StoreError::Precondition { .. }) => {
                let now = self.read_ref_async(name).await?.map(|(id, _)| id);
                Err(conflict(now))
            }
            Err(e) => Err(self.error(&rel, e)),
        }
    }
}

impl Store for S3Store {
    fn location(&self, path: &str) -> PathBuf {
        PathBuf::from(format!("{}/{}", self.origin, self.key(path)))
    }

    fn read(&self, path: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.rt.block_on(self.get(path))?.map(|(bytes, _)| bytes))
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>> {
        let bytes = self
            .rt
            .block_on(self.store.get_range(&self.key(path), offset..offset + len))
            .map_err(|e| self.error(path, e))?;
        if bytes.len() as u64 != len {
            return Err(self.error(path, "the file is shorter than expected"));
        }
        Ok(bytes.to_vec())
    }

    fn list(&self, dir: &str) -> Result<Vec<String>> {
        let listed = self
            .rt
            .block_on(self.store.list_with_delimiter(Some(&self.key(dir))))
            .map_err(|e| self.error(dir, e))?;
        let mut names: Vec<String> = listed
            .common_prefixes
            .iter()
            .chain(listed.objects.iter().map(|o| &o.location))
            .filter_map(|p| p.filename().map(str::to_owned))
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    fn write_new(&self, path: &str, bytes: &[u8]) -> Result<()> {
        let payload = PutPayload::from(bytes.to_vec());
        let options = PutOptions::from(PutMode::Create);
        match self
            .rt
            .block_on(self.store.put_opts(&self.key(path), payload, options))
        {
            // A file already there has the same content (see `Store`).
            Ok(_) | Err(StoreError::AlreadyExists { .. }) => Ok(()),
            Err(e) => Err(self.error(path, e)),
        }
    }

    fn create_dirs(&self, _dirs: &[&str]) -> Result<()> {
        Ok(())
    }

    fn delete(&self, path: &str) -> Result<()> {
        match self.rt.block_on(self.store.delete(&self.key(path))) {
            Ok(()) | Err(StoreError::NotFound { .. }) => Ok(()),
            Err(e) => Err(self.error(path, e)),
        }
    }

    fn read_ref(&self, name: &str) -> Result<Option<ObjectId>> {
        Ok(self
            .rt
            .block_on(self.read_ref_async(name))?
            .map(|(id, _)| id))
    }

    fn list_refs(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let mut dirs = vec![String::new()];
        while let Some(dir) = dirs.pop() {
            let rel = format!("refs{dir}");
            let listed = self
                .rt
                .block_on(self.store.list_with_delimiter(Some(&self.key(&rel))))
                .map_err(|e| self.error(&rel, e))?;
            for prefix in &listed.common_prefixes {
                if let Some(name) = prefix.filename() {
                    dirs.push(format!("{dir}/{name}"));
                }
            }
            for object in &listed.objects {
                if let Some(name) = object.location.filename() {
                    let name = format!("{dir}/{name}");
                    if apeiron_core::validate_ref_name(&name[1..]).is_ok() {
                        names.push(name[1..].to_owned());
                    }
                }
            }
        }
        names.sort();
        Ok(names)
    }

    fn swap_ref(&self, name: &str, expected: Option<ObjectId>, new: ObjectId) -> Result<()> {
        self.rt.block_on(self.swap_ref_async(name, expected, new))
    }
}

#[cfg(test)]
mod tests {
    use object_store::memory::InMemory;

    use super::*;

    fn store() -> S3Store {
        S3Store::new(
            Arc::new(InMemory::new()),
            "backups/apeiron",
            "s3://test".into(),
        )
        .unwrap()
    }

    #[test]
    fn files_are_written_once_and_listed_by_folder() {
        let s = store();
        assert_eq!(s.read("config").unwrap(), None);
        assert!(s.list("").unwrap().is_empty());
        s.write_new("keys/a.key", b"one").unwrap();
        s.write_new("keys/a.key", b"two").unwrap();
        s.write_new("packs/ab/x.pack", b"pack").unwrap();
        assert_eq!(s.read("keys/a.key").unwrap().unwrap(), b"one");
        assert_eq!(s.read_range("packs/ab/x.pack", 1, 2).unwrap(), b"ac");
        assert!(s.read_range("packs/ab/x.pack", 3, 2).is_err());
        assert_eq!(s.list("").unwrap(), ["keys", "packs"]);
        assert_eq!(s.list("keys").unwrap(), ["a.key"]);
        s.delete("keys/a.key").unwrap();
        s.delete("keys/a.key").unwrap();
        assert!(s.list("keys").unwrap().is_empty());
        assert_eq!(
            s.location("config"),
            PathBuf::from("s3://test/backups/apeiron/config")
        );
    }

    #[test]
    fn refs_change_only_from_the_expected_value() {
        let s = store();
        let (a, b) = (ObjectId::from_bytes([1; 32]), ObjectId::from_bytes([2; 32]));
        assert_eq!(s.read_ref("main").unwrap(), None);
        s.swap_ref("main", None, a).unwrap();
        assert!(matches!(
            s.swap_ref("main", None, b),
            Err(Error::RefConflict { actual: Some(x), .. }) if x == a
        ));
        s.swap_ref("main", Some(a), b).unwrap();
        assert_eq!(s.read_ref("main").unwrap(), Some(b));
        assert!(matches!(s.read_ref("Main"), Err(Error::InvalidRefName(_))));
        s.swap_ref("sync/docs", None, a).unwrap();
        s.write_new("refs/main.lock", b"x").unwrap();
        assert_eq!(s.list_refs().unwrap(), ["main", "sync/docs"]);
    }

    #[test]
    fn push_and_pull_through_a_bucket() {
        use apeiron_core::{ChunkerParams, Repository};
        let dir = tempfile::tempdir().unwrap();
        let params = ChunkerParams::new(64 * 1024).unwrap();
        let (mut a, _) = Repository::init_encrypted(&dir.path().join("a"), params, "pw").unwrap();
        let s = store();
        let empty = apeiron_core::Tree { entries: vec![] };
        let mut w = a.writer();
        let tree = w.put_tree(&empty).unwrap();
        let first = w
            .put_snapshot(&apeiron_core::Snapshot {
                tree,
                parents: vec![],
                time: apeiron_core::Time {
                    seconds: 1,
                    nanos: 0,
                },
                author: None,
                host: None,
                message: Some("first".into()),
                meta: vec![],
            })
            .unwrap();
        w.finish().unwrap();
        a.update_ref("main", None, Some(first)).unwrap();

        let report = a.push(&s, "main", &mut |_| Ok(())).unwrap();
        assert_eq!((report.snapshot, report.objects), (first, 2));
        assert_eq!(s.read_ref("main").unwrap(), Some(first));
        assert_eq!(s.list("keys").unwrap().len(), 2);

        // A second copy of the repository pulls from the bucket.
        let b_path = dir.path().join("b");
        std::fs::create_dir(&b_path).unwrap();
        for name in ["config", "keys"] {
            copy(&dir.path().join("a").join(name), &b_path.join(name));
        }
        for name in ["packs", "index", "refs", "logs", "locks", "tmp"] {
            std::fs::create_dir(b_path.join(name)).unwrap();
        }
        let mut b = Repository::unlock(&b_path, "pw").unwrap();
        let report = b.pull(&s, "main", &mut |_| Ok(())).unwrap();
        assert_eq!((report.snapshot, report.objects), (first, 2));
        assert_eq!(
            b.read_snapshot(&first).unwrap().message.as_deref(),
            Some("first")
        );
    }

    fn copy(src: &std::path::Path, dest: &std::path::Path) {
        if src.is_dir() {
            std::fs::create_dir_all(dest).unwrap();
            for entry in std::fs::read_dir(src).unwrap() {
                let entry = entry.unwrap();
                copy(&entry.path(), &dest.join(entry.file_name()));
            }
        } else {
            std::fs::copy(src, dest).unwrap();
        }
    }
}
