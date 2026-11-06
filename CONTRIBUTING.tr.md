> **English:** [CONTRIBUTING.md](CONTRIBUTING.md)

# Apeiron'a Katkı

Bu rehber geliştirme ortamının nasıl kurulacağını, her değişikliğin hangi denetimlerden geçmesi gerektiğini ve değişikliklerin nasıl kabul edildiğini anlatır. Sistemin nasıl kurulduğunu anlamak için önce [docs/ARCHITECTURE.tr.md](docs/ARCHITECTURE.tr.md) belgesini okuyun.

## 1. Depo düzeni

| Yol | İçerik |
| --- | --- |
| `crates/` | Cargo workspace; her katman için bir crate ve işletim sistemi çağrıları için `apeiron-sys` |
| `docs/` | Belgeler; İngilizce (`X.md`) ve Türkçe (`X.tr.md`) |
| `plugins/` | Kendi workspace'i olan metin eklentisi ([docs/TEXT.tr.md](docs/TEXT.tr.md)) |
| `scripts/` | Performans ölçümü ([docs/PERFORMANCE.tr.md](docs/PERFORMANCE.tr.md)) |
| `sdk/` | C örneği, Python SDK'sı ve ortak uyumluluk testleri ([docs/SDK.tr.md](docs/SDK.tr.md)) |
| `.github/workflows/` | CI: biçim, lint ve testler |
| `LICENSE-MIT`, `LICENSE-APACHE` | İki lisans |

## 2. Geliştirme ortamı

- **Rust:** [rustup](https://rustup.rs) ile kurulur (`rustfmt` ve `clippy` içeren kararlı araç zinciri). Desteklenen en düşük sürüm 1.97'dir (2024 sürümü). Eklenti barındırıcısının testleri için ayrıca `rustup target add wasm32-wasip2` gerekir.
- **C derleyicisi:** `zstd`, `blake3` ve `aws-lc-rs` gibi bağımlılıkları derlemek için gerekir (Linux/macOS'ta GCC ya da Clang, Windows'ta MSVC).
- **Linux'ta masaüstü uygulaması:** WebKitGTK dosyaları (Debian ve Ubuntu'da `libwebkit2gtk-4.1-dev`).
- **Platformlar:** Linux (x86_64, aarch64), macOS ve Windows'un hepsi CI'da test edilir.

```bash
git clone https://github.com/efe-clk/apeiron.git
cd apeiron
cargo build
cargo test
```

## 3. Her değişiklikten önce denetimler

Her değişiklik bu üç komuttan temiz geçmelidir. CI bunları her push ve pull request'te çalıştırır.

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

| Denetim | Kural |
| --- | --- |
| Biçim | Varsayılan ayarlarla `cargo fmt` |
| Lint | Hiç Clippy uyarısı yok; uyarılar hata sayılır |
| Testler | Tüm testler Linux x86_64, Linux aarch64, macOS ve Windows'ta geçer |

## 4. Kod kuralları

| Kural | Ayrıntı |
| --- | --- |
| Mümkün olan her yerde Rust | C yalnızca dışa açılan ABI katmanında ([ADR-001](docs/DECISIONS.tr.md#adr-001-ana-dil-olarak-rust)) |
| Katı katmanlama | Her katman ayrı bir crate'tir ve yalnızca hemen altındaki katmanı kullanır |
| `unsafe` yok | `unsafe` yalnızca FFI crate'inde ve işletim sistemi çağrılarını tutan `apeiron-sys` içinde serbesttir ([ADR-014](docs/DECISIONS.tr.md#adr-014-güvensiz-sistem-çağrıları-için-küçük-bir-sistem-cratei)); diğer her crate'te `#![forbid(unsafe_code)]` vardır |
| Tipli hatalar | Kütüphane crate'leri `thiserror` kullanır; kütüphane kodunda `unwrap`/`expect` yoktur (testlerde kullanılabilir) |
| Hata enjeksiyonu testleri | Veri bütünlüğü kodu (yazma, ref güncelleme, kurtarma) hata enjeksiyonu testleriyle doğrulanır |
| Kodda İngilizce | Kod, tanımlayıcılar, yorumlar ve commit mesajları İngilizcedir |
| Çevreye uyum | Yeni kod, çevresindeki kodun adlandırmasına, yorum yoğunluğuna ve alışkanlıklarına uyar |

## 5. Commit mesajları

Commit'ler [Conventional Commits](https://www.conventionalcommits.org/) kuralına uyar:

```text
<tür>(<isteğe bağlı kapsam>): <emir kipinde kısa özet>

<isteğe bağlı gövde: ne ve neden>
```

| Tür | Ne zaman |
| --- | --- |
| `feat` | Yeni bir yetenek |
| `fix` | Hata düzeltmesi |
| `docs` | Yalnızca belge |
| `test` | Yalnızca test |
| `refactor` | Ne hata düzelten ne de özellik ekleyen kod değişikliği |
| `build` | Derleme sistemi ya da bağımlılıklar |
| `ci` | CI yapılandırması |
| `chore` | Diğer her şey |

Örnek: `feat(core): add storage engine with packs, index and atomic refs`.

## 6. Depolama formatı değişiklikleri

Depolama formatı herkese açık bir sözleşmedir ([docs/FORMAT.tr.md](docs/FORMAT.tr.md)): bugün yazılan veri okunabilir kalmalıdır.

1. Formatı etkileyen her değişiklik **önce** `docs/FORMAT.md` ve `docs/FORMAT.tr.md` içinde belgelenir.
2. Değişiklik üzerinde uzlaşıldıktan sonra kodlanır.
3. Mevcut okuyucuların anlayamayacağı bir değişiklik format sürümünü yükseltir; eski sürümler okunabilir kalmalıdır.
4. Değişiklik bir karar ise `docs/DECISIONS.md` ve `docs/DECISIONS.tr.md` belgelerine yeni bir ADR eklenir.

## 7. Belgeler

Belgeler iki dillidir: `X.md` İngilizce, `X.tr.md` Türkçedir.

| Kural | Ayrıntı |
| --- | --- |
| Aynı commit | Bir belge değişince iki dil sürümü aynı commit'te güncellenir |
| Aynı yapı | İki sürümde aynı başlıklar, tablolar ve diyagramlar bulunur |
| Dil bağlantısı | Her dosya, öbür dildeki sürümüne giden bir bağlantıyla başlar |
| Testle denetlenir | `cargo test`, `crates/apeiron-cli/tests/docs.rs` dosyasını çalıştırır: iki dil ve yapıları, bağlantılar, bölüm atıfları, programın her komutu ve seçeneği, C ABI'nin her işlemi, her crate, en düşük Rust sürümü ve her aşamanın durumu |
| Dürüst durum | Yazılmamış özellikler hiçbir zaman çalışıyor gibi anlatılmaz; durum her zaman belirtilir (Planlandı / Devam ediyor / Tamamlandı) |
| Diyagramlar | Diyagramlar GitHub'da çizim olarak görünsün diye Mermaid ile yazılır |
| Bağlantılar | İç bağlantılar göreli yol kullanır |

## 8. Katkıların lisansı

Aksini açıkça belirtmediğiniz sürece gönderdiğiniz her katkı, ek koşul olmadan MIT OR Apache-2.0 çift lisansıyla sunulmuş sayılır ([ADR-008](docs/DECISIONS.tr.md#adr-008-çift-lisans-mit-or-apache-20)).
