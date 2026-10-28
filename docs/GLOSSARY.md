> **Türkçe:** [GLOSSARY.tr.md](GLOSSARY.tr.md)

# Apeiron Glossary

Terms used across the Apeiron documentation and code, with their Turkish equivalents. Where the Turkish documents keep the English word (for example *chunk* or *snapshot*), the Turkish column says so.

## 1. Object model

| Term | Turkish | Definition |
| --- | --- | --- |
| Blob | blob | The ordered list of chunks that make up one file's content |
| Chunk | chunk (parça) | A content-defined piece of a file, about 1 MiB, addressed by its hash |
| Object | nesne | Any stored unit: chunk, blob, tree or snapshot. Immutable once written |
| Object id | nesne kimliği | The 32-byte identifier of an object: its keyed BLAKE3 hash |
| Object type | nesne türü | One byte that says whether an object is a chunk, blob, tree or snapshot |
| Parent snapshot | ebeveyn snapshot | A previous snapshot that a snapshot builds on; forms the history |
| Policy | politika (policy) | Rules for retention, encryption, sync and verification, kept separate from data |
| Ref | ref | A name pointing at a snapshot; branches, tags and backup points are all refs |
| Reflog | reflog | The append-only change log of one ref; used to undo ref changes |
| Repository | depo | A directory holding a `config` file, packs, indexes, refs and reflogs |
| Snapshot | snapshot (anlık görüntü) | The state of a data set at one moment, with parents, author and message |
| Store | store (depolama hedefi) | Where objects are kept: local disk, external disk, network or remote target |
| Tree | tree (dizin ağacı) | One directory: entries with names, kinds, permissions, times and attributes |
| Type driver | type driver (tür sürücüsü) | A plugin that adds type-specific diff, merge, preview and indexing |

## 2. Storage and integrity

| Term | Turkish | Definition |
| --- | --- | --- |
| Append-only | yalnızca eklemeli | Storage that accepts new data but never changes or deletes existing data |
| Atomic rename | atomik yeniden adlandırma | Replacing a file in one step, so readers see either the old or the new file |
| BLAKE3 | BLAKE3 | The cryptographic hash algorithm used for all hashing in Apeiron |
| Canonical encoding | kanonik kodlama | An encoding where the same value always gives the same bytes |
| CBOR | CBOR | Concise Binary Object Representation (RFC 8949); used for blobs, trees and snapshots |
| Compare-and-swap (CAS) | karşılaştır ve değiştir | Updating a value only if it still has the expected old value |
| Content addressing | içerik adresleme | Naming data by the hash of its content |
| Content-defined chunking (CDC) | içerik tanımlı bölme | Choosing chunk boundaries from the data itself so that edits move few boundaries |
| Corruption | bozulma | Stored data that no longer matches its hash or structure |
| Deduplication | tekilleştirme | Storing identical data only once |
| FastCDC | FastCDC | The content-defined chunking algorithm Apeiron uses (2020 version) |
| Fault injection | hata enjeksiyonu | Testing by deliberately interrupting or breaking an operation at a chosen point |
| `fsync` | `fsync` (diske sabitleme) | Forcing written data out of caches onto the disk |
| Garbage collection | çöp toplama | Removing objects that no ref or reflog reaches any more |
| Hash | özet (hash) | A fixed-size fingerprint computed from data |
| Id key | kimlik anahtarı | The random per-repository key used for keyed object ids |
| Index file | indeks dosyası | A file that records which object is in which pack; a rebuildable cache |
| Integrity | bütünlük | The guarantee that data is exactly what was stored |
| Keyed hash | anahtarlı özet | A hash that also depends on a secret key |
| Lock file | kilit dosyası | A file created exclusively to stop two writers changing a ref at once |
| Pack file | paket dosyası | A file holding many objects, named by its own BLAKE3 hash |
| Scrub | tam doğrulama (scrub) | A full, scheduled verification of every stored object |
| Torn write | yarım yazma | A write cut off by a crash or power failure, leaving partial data |
| zstd | zstd | The compression algorithm used inside pack files |

## 3. Operations

| Term | Turkish | Definition |
| --- | --- | --- |
| Archive | arşiv | A single portable package of a data set or version |
| Backup | yedek | An independent copy of data for recovery |
| Branch | dal | A ref that moves as new snapshots are added on top of it |
| Cherry-pick | seçerek taşıma (cherry-pick) | Applying one selected change from one line of history to another |
| Conflict | çatışma | Two changes to the same data that cannot be combined automatically |
| Export | dışa aktarma | Writing data out in another format (plain files, tar/zip, Git) |
| Import | içe aktarma | Bringing data in from another format or system |
| Incremental backup | artımlı yedek | A backup that transfers only what changed |
| Merge | birleştirme | Combining two lines of history into one |
| Rebase | yeniden temellendirme (rebase) | Replaying changes on top of a different starting point |
| Restore | geri yükleme | Writing data from a snapshot back to files |
| Restore test | geri yükleme testi | Regularly restoring a backup to a separate area to prove it works |
| Retention policy | saklama politikası | Rules for how many backups to keep and for how long |
| Sync | senkron (senkronizasyon) | Keeping data the same on several devices or targets |
| Tag | etiket | A ref that names a fixed snapshot |

## 4. Architecture and tooling

| Term | Turkish | Definition |
| --- | --- | --- |
| ADR | ADR (mimari karar kaydı) | Architecture decision record: a short document for one decision |
| C ABI | C ABI | A stable binary interface in C that other languages can call |
| Cargo workspace | Cargo workspace (çalışma alanı) | A set of Rust crates built and tested together |
| CLI | CLI (komut satırı arayüzü) | Command-line interface |
| Client-side encryption | istemci tarafı şifreleme | Encrypting data on the user's device before it leaves |
| Component model | bileşen modeli | The WebAssembly standard for typed, language-neutral components |
| Conventional Commits | Conventional Commits | A commit message convention (`feat:`, `fix:`, `docs:` …) |
| Crate | crate | A Rust package: a library or a program |
| Daemon | arka plan hizmeti (daemon) | A program that runs in the background, e.g. for scheduled backups |
| FFI | FFI | Foreign function interface: calling code across languages |
| gitoxide (`gix`) | gitoxide (`gix`) | A Git implementation written in Rust |
| Hook | hook (kanca) | A plugin that runs when an event happens, e.g. after a snapshot |
| Importer / exporter | içe / dışa aktarıcı | A plugin that moves data from or to another system |
| MSRV | MSRV (en düşük Rust sürümü) | Minimum supported Rust version |
| Plugin | eklenti | Optional code that extends Apeiron without changing the core |
| Recovery key | kurtarma anahtarı | A random key shown once when an encrypted repository is created; it opens the repository when every password is lost |
| Sandbox | sandbox (yalıtım ortamı) | An isolated environment where a plugin can only do what it is allowed to |
| SDK | SDK | Software development kit: a library for using Apeiron from a language |
| Storage backend | storage backend (depolama arka ucu) | A plugin that adds a new storage target |
| Tokio | Tokio | The asynchronous runtime used in the network, sync and daemon layers |
| Wasmtime | Wasmtime | The WebAssembly runtime that runs plugins |
| WebAssembly (WASM) | WebAssembly (WASM) | A portable binary format that runs in a sandbox |
| WTF-8 | WTF-8 | An encoding used to store Windows file names as bytes |
