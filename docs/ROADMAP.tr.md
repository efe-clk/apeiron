> **English:** [ROADMAP.md](ROADMAP.md)

# Apeiron Yol Haritası

Pahalı kararlar (format, nesne modeli, API ve eklenti sınırı) Aşama 0'da verilir; geri kalan her şey bu temelin üstüne kurulur. Her aşama kendi başına kullanılabilir bir ürün bırakır. Aşamalar ve çıkış kriterleri [VISION.tr.md](VISION.tr.md) belgesinden alınmıştır.

**Şu anki aşama: Aşama 5 — Kurumsal (Devam ediyor).** Aşama 0 ve Aşama 1 2026-10-28'de, Aşama 2 ve Aşama 3 2026-10-29'da, Aşama 4 2026-10-30'da tamamlandı; Aşama 1'in ve Aşama 2'nin bir aylık denemeleri proje sahibinin kararıyla atlandı.

## 1. Genel bakış

```mermaid
flowchart LR
    S0["Aşama 0<br/>Format ve çekirdek<br/>Tamamlandı"]
    S1["Aşama 1<br/>Yerel MVP<br/>Tamamlandı"]
    S2["Aşama 2<br/>Uzak ve senkron<br/>Tamamlandı"]
    S3["Aşama 3<br/>Geliştirici ve ekosistem<br/>Tamamlandı"]
    S4["Aşama 4<br/>Arayüzler ve çok kullanıcı<br/>Tamamlandı"]
    S5["Aşama 5<br/>Kurumsal<br/>Devam ediyor"]
    S0 --> S1 --> S2 --> S3 --> S4 --> S5
```

| Aşama | Kapsam | Çıkış kriteri | Durum |
| --- | --- | --- | --- |
| 0. Format ve çekirdek | Nesne modeli ve format spesifikasyonu, depolama motoru, bütünlük doğrulama, Rust API taslağı | Spesifikasyon yazılı; güç kesintisi testlerinde veri kaybı yok | Tamamlandı |
| 1. Yerel MVP | CLI; snapshot alma, geçmiş, dosya düzeyi diff, geri yükleme, doğrulama; yerel ve harici disk yedek hedefi; tar, zip ve 7z dışa aktarma | Kendi verini bir ay boyunca yalnızca Apeiron ile yedekleyip geri yükleyebilmek | Tamamlandı (bir aylık deneme atlandı) |
| 2. Uzak ve senkron | Arka plan hizmeti, zamanlanmış yedek, şifreleme, SFTP ve S3 hedefleri, tek/çift yönlü senkron, çatışma tespiti, geri yükleme testi | İki cihaz ve bir uzak sunucu arasında bir ay sorunsuz senkron | Tamamlandı (bir aylık deneme atlandı) |
| 3. Geliştirici ve ekosistem | Dal ve birleştirme, Git içe/dışa aktarma (hedef: Git ağ protokolü), C ABI ve Python SDK, WebAssembly eklenti sistemi, metin diff eklentisi, arama | Gerçek bir Git deposu Apeiron'a taşınıp geri alınabiliyor | Tamamlandı |
| 4. Arayüzler ve çok kullanıcı | Masaüstü ve web arayüzü, kullanıcı ve izinler, denetim kaydı, paylaşım | Teknik olmayan bir kullanıcı CLI'sız yedek alıp geri yükleyebiliyor | Tamamlandı |
| 5. Kurumsal | Merkezi kimlik, çok kiracılı kullanım, replikasyon, yüksek erişilebilirlik, değiştirilemez arşiv, uyumluluk raporları | İlk kurumsal pilot kurulum | Devam ediyor |

## 2. Aşama 0 — Format ve çekirdek

**Çıkış kriteri:** Spesifikasyon yazılı; güç kesintisi testlerinde veri kaybı yok.

| # | İş | Durum | Not |
| --- | --- | --- | --- |
| 1 | Depolama formatı spesifikasyonu ([FORMAT.tr.md](FORMAT.tr.md)) | Tamamlandı | Sürüm 1, 2026-10-27'de onaylandı |
| 2 | Cargo workspace iskeleti ve CI (fmt, clippy, test) | Tamamlandı | Linux x86_64/aarch64, macOS, Windows |
| 3 | Depolama motoru: chunk yazma/okuma, BLAKE3 doğrulama, atomik ref güncelleme | Tamamlandı | `apeiron-core` içinde paketler, indeksler, ref'ler ve reflog'lar |
| 4 | Blob, tree ve snapshot'ın kanonik CBOR kodlaması ([FORMAT.tr.md](FORMAT.tr.md) §6) | Tamamlandı | `minicbor` ile; kanonik olmayan nesneler reddedilir |
| 5 | Güç kesintisi ve bozulma testleri ([FORMAT.tr.md](FORMAT.tr.md) §10) | Tamamlandı | Diskteki her değişiklikte çökme ve disk dolması taklit edilir; bit bozulması bir paketin her baytında denenir |
| 6 | Rust API taslağı | Tamamlandı | [API.tr.md](API.tr.md) §8, `apeiron-core` içinde uygulandı; ayrı `apeiron` crate'i CLI ile gelecek |

Windows'ta yeniden adlandırmalar küçük `apeiron-sys` crate'i üzerinden `MOVEFILE_WRITE_THROUGH` kullanır ([FORMAT.tr.md](FORMAT.tr.md) §9.1, [ADR-014](DECISIONS.tr.md#adr-014-güvensiz-sistem-çağrıları-için-küçük-bir-sistem-cratei)).

## 3. Aşama 1 — Yerel MVP

**Çıkış kriteri:** Kendi verini bir ay boyunca yalnızca Apeiron ile yedekleyip geri yükleyebilmek.

| İş | Durum |
| --- | --- |
| CLI (tüm işlemler, JSON çıktı, ilerleme, iptal) | Tamamlandı |
| Snapshot alma, geçmiş, dosya düzeyi diff | Tamamlandı |
| Geri yükleme ve doğrulama (scrub) | Tamamlandı |
| Yerel ve harici disk yedek hedefleri | Tamamlandı |
| tar, zip ve 7z dışa aktarma | Tamamlandı |
| Çöp toplama, depo kilitleri, paket birleştirme | Tamamlandı |
| İndeksi bozuk bir depoyu, indeksi paketlerden yeniden üreterek açmak | Tamamlandı |

## 4. Aşama 2 — Uzak ve senkron

**Çıkış kriteri:** İki cihaz ve bir uzak sunucu arasında bir ay sorunsuz senkron.

İşler bu sırayla yapılır, çünkü her biri bir öncekinin üzerine kurulur: uzak hedefler yalnızca şifreli veri almalıdır ve arka plan hizmeti önceki işlerin sağladıklarını zamanlar.

| # | İş | Durum |
| --- | --- | --- |
| 1 | İstemci tarafı şifreleme ([FORMAT.tr.md](FORMAT.tr.md) §14) | Tamamlandı |
| 2 | Uzak hedefler (klasör, SFTP, S3), push ve pull ile ([FORMAT.tr.md](FORMAT.tr.md) §15) | Tamamlandı |
| 3 | Arka plan hizmeti (daemon), zamanlanmış yedek, geri yükleme testi ([DAEMON.tr.md](DAEMON.tr.md)) | Tamamlandı |
| 4 | Tek ve çift yönlü senkron, çatışma tespiti ([SYNC.tr.md](SYNC.tr.md)) | Tamamlandı |

## 5. Aşama 3 — Geliştirici ve ekosistem

**Çıkış kriteri:** Gerçek bir Git deposu Apeiron'a taşınıp geri alınabiliyor.

Önce çıkış kriteri geldi; 2026-10-29'da 1. işle karşılandı ve aşama aynı gün diğer işlerle tamamlandı. Eklenti sistemi, ilk kullanıcısı olan metin diff eklentisinden önce gelir.

| # | İş | Durum |
| --- | --- | --- |
| 1 | Git içe/dışa aktarma ([GIT.tr.md](GIT.tr.md); Git ağ protokolü hedef olarak kalıyor) | Tamamlandı |
| 2 | Dal ve birleştirme ([BRANCH.tr.md](BRANCH.tr.md)) | Tamamlandı |
| 3 | Arama ([SEARCH.tr.md](SEARCH.tr.md)) | Tamamlandı |
| 4 | C ABI ve Python SDK ([SDK.tr.md](SDK.tr.md)) | Tamamlandı |
| 5 | WebAssembly eklenti sistemi ([PLUGIN.tr.md](PLUGIN.tr.md)) | Tamamlandı |
| 6 | Metin diff eklentisi ([TEXT.tr.md](TEXT.tr.md)) | Tamamlandı |

## 6. Aşama 4 — Arayüzler ve çok kullanıcı

**Çıkış kriteri:** Teknik olmayan bir kullanıcı CLI'sız yedek alıp geri yükleyebiliyor.

Masaüstü arayüzü önce gelir, çünkü çıkış kriterini tek başına karşılar; web arayüzü onun sayfalarını yeniden kullanır; kullanıcılar ve izinler, onları kaydeden ve kullanan denetim kaydı ile paylaşımdan önce gelir.

| # | İş | Durum |
| --- | --- | --- |
| 1 | Masaüstü arayüzü ([DESKTOP.tr.md](DESKTOP.tr.md)) | Tamamlandı |
| 2 | Web arayüzü ([WEB.tr.md](WEB.tr.md)) | Tamamlandı |
| 3 | Kullanıcılar ve izinler ([USERS.tr.md](USERS.tr.md)) | Tamamlandı |
| 4 | Denetim kaydı ([AUDIT.tr.md](AUDIT.tr.md)) | Tamamlandı |
| 5 | Paylaşım ([SHARE.tr.md](SHARE.tr.md)) | Tamamlandı |

## 7. Aşama 5 — Kurumsal

**Çıkış kriteri:** İlk kurumsal pilot kurulum.

Merkezi kimlik önce gelir, çünkü pilot bir kurum her şeyden önce çalışanlarını kendi kimlik sağlayıcısı üzerinden oturum açtırır.

| # | İş | Durum |
| --- | --- | --- |
| 1 | Merkezi kimlik ([IDENTITY.tr.md](IDENTITY.tr.md)) | Tamamlandı |
| 2 | Çok kiracılı kullanım ([TENANTS.tr.md](TENANTS.tr.md)) | Tamamlandı |
| 3 | Replikasyon ve yüksek erişilebilirlik | Planlandı |
| 4 | Değiştirilemez arşiv | Planlandı |
| 5 | Uyumluluk raporları | Planlandı |
