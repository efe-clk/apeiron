> **English:** [API.md](API.md)

# Apeiron Herkese Açık API — Tasarım İlkeleri

> **Durum: Devam ediyor.** Bölüm 1–7 API'nin ilkeleridir, her biri durumunu belirtir; bölüm 8, uygulanmış olan herkese açık `apeiron` crate'inin spesifikasyonudur; bölüm 9 zaten var olan çekirdek işlevleri listeler. C ABI ve SDK'lar [SDK.tr.md](SDK.tr.md) içinde tanımlanır.

Herkese açık API, Apeiron'a girmenin tek yoludur. CLI, arka plan hizmeti, masaüstü ve web arayüzleri ve tüm dış uygulamalar onu kullanır; gizli iç kapı yoktur ([VISION.tr.md](VISION.tr.md), ilke 8).

## 1. Katmanlar

```mermaid
flowchart TB
    APPS["Uygulamalar<br/>CLI · Arka plan hizmeti · Masaüstü · Web · Üçüncü taraf"]
    SDK["SDK'lar<br/>Python · C++ · Go · C# · Java/Kotlin · Swift · JS/TS · Zig"]
    CABI["C ABI<br/>kararlı, sürümlü"]
    RAPI["Rust API<br/>herkese açık crate"]
    SVC["Hizmetler"]
    APPS --> SDK --> CABI --> RAPI --> SVC
    APPS --> RAPI
```

| Katman | Rol | Durum |
| --- | --- | --- |
| Rust API | Tek doğruluk kaynağı; her işlem burada tanımlanır | Devam ediyor (`apeiron` crate'i, bölüm 8) |
| C ABI | Rust API üzerinde kararlı bir C arayüzü; `unsafe` kullanabilen iki crate'ten biri (`apeiron-sys` ile birlikte) | Tamamlandı (`apeiron-ffi`) |
| SDK'lar | C ABI'yi saran, her dilin alışkanlıklarına uyan ince katmanlar | Devam ediyor (Python tamamlandı) |

SDK'lar mantığı yeniden yazmaz, yalnızca C ABI'yi sarar. Aynı işlemin her dilde aynı sonucu vermesi böyle sağlanır.

## 2. İşlem modeli

| İlke | Açıklama | Durum |
| --- | --- | --- |
| Tek model, iki kullanım | Aynı işlemler ve türler gömülü kullanımda (aynı süreçte kütüphane) ve uzak kullanımda (arka plan hizmeti üzerinden) çalışır | Devam ediyor (gömülü tamamlandı; uzak zamanlanmadı, [ROADMAP.tr.md](ROADMAP.tr.md) §8) |
| Eksiksiz kapsam | Sürümleme, arşiv, yedek, senkron ve yönetim işlemlerinin hepsi erişilebilir; CLI'ın yapabildiği her şeyi API de yapabilir | Devam ediyor (bölüm 8.3; `web` komutları sunucunun kendi dosyalarını yönetir, API'de değildir) |
| Açık sonuç | Her kritik işlem açık bir başarıyla ya da tipli bir hatayla biter; "muhtemelen oldu" durumu yoktur | Tamamlandı |
| Düz veri giriş ve çıkışları | İstekler ve sonuçlar, serileştirilebilen düz veri türleridir (uzak kullanım ve C ABI için) | Tamamlandı |

Uzun işlemler (yedek, geri yükleme, senkron, doğrulama) **iş** (job) olarak başlatılır. Bir iş izlenebilir, iptal edilebilir ve sonucu sorgulanabilir (bölüm 8.5).

## 3. İlerleme ve iptal

| İlke | Açıklama | Durum |
| --- | --- | --- |
| İlerleme bildirimi | Uzun işler ilerlemeyi bildirir (biten ve toplam öğe ve bayt, o anki evre) | Tamamlandı |
| İptal | Her uzun iş iptal edilebilir; iptal, depoyu geçerli bir durumda bırakır (eski ya da yeni, asla yarım) | Tamamlandı |
| Zaman aşımı | SDK'lar aynı iptal mekanizmasıyla zaman aşımını destekler | Tamamlandı (Python) |
| Sonuç sorgulama | Biten bir işin sonucu sonradan okunabilir; uzak kullanımda da | Gömülü kullanım ve web sunucusu için tamamlandı; uzak kullanım henüz yok |

İptal, tasarım gereği güvenlidir: nesneler ancak diske sabitlendikten sonra görünür ve ref'ler en son güncellenir ([FORMAT.tr.md](FORMAT.tr.md) §9).

## 4. Akış

| İlke | Açıklama | Durum |
| --- | --- | --- |
| Tampon değil akış | Dosya içeriği akış olarak girer ve çıkar (Rust'ta `Read`/`Write`); içerik hiçbir zaman tamamen belleğe alınmaz | Rust API'de tamamlandı; C ABI yollarla çalışır ([SDK.tr.md](SDK.tr.md) §7) |
| Sınırlı bellek | Bellek kullanımı dosya boyutundan bağımsızdır; hedef, Raspberry Pi 4'te 10 GB'lık bir dosya eklerken 256 MB altı ([VISION.tr.md](VISION.tr.md)) | Buna göre tasarlandı; henüz ölçülmedi ([ROADMAP.tr.md](ROADMAP.tr.md) §8) |
| Tembel listeleme | Geçmiş, tree'ler ve arama sonuçları tek büyük liste olarak değil, yineleyici (iterator) ya da sayfalar olarak döner | Devam ediyor (geçmiş sayfalıdır; tree'ler ve arama sonuçları listedir, arama sınırlıdır) |

## 5. Hata türleri

| İlke | Açıklama | Durum |
| --- | --- | --- |
| Tipli hatalar | Hatalar metin değil, tipli enum'lardır (`thiserror`) | `apeiron-core` içinde tamamlandı |
| Bozulma ayrı bir hatadır | Doğrulamadan geçemeyen veri ayrı bir bozulma hatası üretir ve hiçbir zaman döndürülmez | `apeiron-core` içinde tamamlandı |
| Eşzamanlılık hataları açıktır | "Kilitli" ve "çakışma" (karşılaştır-ve-değiştir başarısız) ayrı hatalardır; çağıran yeniden deneyebilir ya da kullanıcıya sorabilir | `apeiron-core` içinde tamamlandı |
| Genişletilebilir | Hata enum'ları `#[non_exhaustive]` işaretlidir; yeni hatalar çağıranları bozmadan eklenebilir | `apeiron-core` içinde tamamlandı |
| Kararlı kodlar | Her hatanın C ABI ve SDK'lar için kararlı bir sayısal kodu vardır | Tamamlandı |
| Sınırda panik yok | Panik hiçbir zaman C ABI'yi geçmez; bir hata koduna dönüşür | Tamamlandı |

## 6. Sürümleme politikası

| İlke | Açıklama | Durum |
| --- | --- | --- |
| Semantik sürümleme | Rust API, C ABI ve her SDK SemVer'e uyar | Planlandı |
| 1.0 öncesi | Küçük sürümler arasında uyumu bozan değişikliklere izin verilir ve sürüm notlarında listelenir | Planlandı |
| 1.0'dan sonra | Uyumu bozan değişiklik yalnızca yeni ana sürümde, yazılı bir uyumluluk politikasıyla | Planlandı |
| Ayrı sürümler | Depolama formatının ([FORMAT.tr.md](FORMAT.tr.md)), C ABI'nin ve eklenti API'sinin kendi sürüm numaraları vardır | Tamamlandı: format sürümü 1, C ABI 0.1, eklenti API'si 0.1 |
| En düşük Rust sürümü | Workspace desteklenen en düşük Rust sürümünü belirtir (şu an 1.97) | Tamamlandı |

## 7. C ABI ve SDK katmanı

| İlke | Açıklama | Durum |
| --- | --- | --- |
| Opak tutamaçlar | Depolar, işler ve yineleyiciler, açık serbest bırakma işlevleri olan opak işaretçilerdir (opaque handle) | Tamamlandı |
| Hata kodları ve mesajlar | Her işlev bir durum kodu döndürür; mesajı ayrı bir çağrı verir | Tamamlandı |
| Sahiplik açıktır | Her işlev hangi belleği kimin serbest bırakacağını belgeler | Tamamlandı |
| Sorgulanan ilerleme | İlerleme bir işlev çağrısıyla okunur, iş bir iptal çağrısıyla durdurulur; sınırı hiçbir geri çağrı (callback) geçmez (bölüm 8.5) | Tamamlandı |
| Üretilen başlık dosyası | C başlık dosyası Rust kodundan üretilir ve CI'da denetlenir | Tamamlandı |
| Ortak test paketi | Aynı senaryolar her SDK'da çalıştırılır ve sonuçlar karşılaştırılır | Tamamlandı |

| SDK dalgası | Diller | Durum |
| --- | --- | --- |
| İlk | Rust (yerel), C (ABI), Python | Tamamlandı |
| İlk | C++ | Planlandı |
| Sonraki | Go, C#, Java/Kotlin, Swift, JavaScript/TypeScript, Zig | Planlandı |

## 8. Rust API spesifikasyonu

Bu bölüm herkese açık `apeiron` crate'inin 0.1 sürümlü sözleşmesidir. Aşama 0 taslağının çekirdek işlevleri (bölüm 9) bunun altındaki yapı taşlarıdır ve sözleşmenin parçası değildir.

### 8.1 Crate'ler

```mermaid
flowchart TB
    CLI["apeiron-cli<br/>apeiron programı"]
    FFI["apeiron-ffi<br/>C ABI"]
    API["apeiron<br/>herkese açık Rust API"]
    SVC["apeiron-services<br/>yedek, geri yükleme, fark, dışa aktarma"]
    CORE["apeiron-core<br/>nesneler, paketler, ref'ler"]
    SYS["apeiron-sys<br/>sistem çağrıları"]
    CLI --> API
    FFI --> API
    API --> SVC --> CORE --> SYS
```

| Crate | Görev | Durum |
| --- | --- | --- |
| `apeiron` | Uygulamaların kullandığı tek crate: depo tutamacı, işlemler, işler, hatalar | Devam ediyor |
| `apeiron-services` | Klasörleri tree ve snapshot'a, onları da geri klasöre çevirir; fark; tar, zip ve 7z dışa aktarma | Devam ediyor |
| `apeiron-cli` | `apeiron` komutu ([CLI.tr.md](CLI.tr.md)) | Devam ediyor |
| `apeiron-ffi` | `apeiron` üzerinde C ABI ([SDK.tr.md](SDK.tr.md)) | Tamamlandı |
| `apeiron-git`, `apeiron-plugin`, `apeiron-remote` | Git içe ve dışa aktarma, eklenti barındırıcısı, SFTP ve S3 uzak depoları; hepsini `apeiron` kullanır | Tamamlandı |

### 8.2 Depo tutamacı

| İşlev | Ne yapar |
| --- | --- |
| `Repository::init(path)` | Boş bir depo oluşturur ve açar |
| `Repository::init_encrypted(path, password)` | Boş, şifreli bir depo oluşturur ([FORMAT.tr.md](FORMAT.tr.md) §14), açar ve kurtarma anahtarını döndürür; bu anahtar hiçbir yerde saklanmaz ve kullanıcıya bir kez gösterilir |
| `Repository::open(path)` | Var olan bir depoyu açar; şifreli bir depoda `Key` hatası verir |
| `Repository::unlock(path, secret)` | Bir depoyu bir parola ya da kurtarma anahtarıyla açar |
| `Repository::clone_from(request)` | Uzun: yeni ya da boş bir klasörde, bir uzak kopyadan yeni bir depo oluşturur (URL `add_remote`'taki gibi), her ref'i pull eder ve uzak depoyu `origin` olarak ekler ([FORMAT.tr.md](FORMAT.tr.md) §15.6); şifreli bir kopyanın parolası istekle verilir. `CloneReport` döndürür: her ref için bir `TransferReport` |
| `repo.info()` | Yolu, format sürümünü, chunk bölme ayarlarını, deponun şifreli olup olmadığını ve açılırken bozuk bulunan dosyaları döndürür |

Bir `Repository` iş parçacıkları arasında paylaşılabilir (`Send + Sync`). Yalnızca okuyan işlemler paralel çalışır; yazan işlemler süreç içinde birbirini bekler. Her işlem çalışırken depo kilidini paylaşımlı, çöp toplama ise özel tutar ([FORMAT.tr.md](FORMAT.tr.md) §12); böylece çöp toplama hiçbir zaman başka bir programla aynı anda çalışmaz. Diğer yazıcılar aynı anda çalışabilir, çünkü ref'ler karşılaştır ve değiştir ile güncellenir.

### 8.3 İşlemler

Her işlem tek bir düz istek değeri alır ve tek bir düz sonuç değeri döndürür. Uzun işlemler bir iş döndürür (bölüm 8.5).

| İşlem | İstek | Sonuç | Uzun |
| --- | --- | --- | --- |
| `backup` (`exclude`: dışarıda bırakılacak kalıplar, [CLI.tr.md](CLI.tr.md) §4.1; sonuçta `excluded`) | kaynak klasör, ref adı, açıklama, yazar, cihaz, meta | `BackupReport`: snapshot kimliği, dosya, klasör ve bayt sayıları, yeni saklanan baytlar, uyarılar | Evet |
| `restore` | snapshot, hedef klasör, isteğe bağlı snapshot içi yol | `RestoreReport`: sayılar, uyarılar | Evet |
| `export` | snapshot, çıktı dosyası, biçim (`tar`, `zip` ya da `7z`; verilmezse uzantıdan), isteğe bağlı yol | `ExportReport`: sayılar, uyarılar | Evet |
| `verify` | — | `VerifyReport`: denetlenen nesneler, bozuk dosyalar ve nesneler | Evet |
| `restore_test` | snapshot, geri yüklenecek bayt (varsayılan 100 MiB) | `RestoreTestReport`: geri yüklenip denetlenen dosyalar ve baytlar, bozuk dosyalar | Evet |
| `log` | başlangıç snapshot'ı, en fazla sayı, isteğe bağlı olarak mesajın, yazarın, makinenin ya da üst verinin içermesi gereken metin (büyük-küçük harf ayrılmaz; sayı o zaman eşleşenleri sayar) | `LogPage`: en yeniden başlayan snapshot'lar ve sonraki sayfanın başladığı yer | Hayır |
| `list` | snapshot, yol | `Vec<EntryInfo>`: ad, tür, boyut, izinler, değişiklik zamanı | Hayır |
| `read_file` | snapshot, bir dosyanın yolu | dosyanın boyutu; içeriği okundukça bir `Write` içine yazılır, bu yüzden JSON API'de yoktur ([SHARE.tr.md](SHARE.tr.md)) | Hayır |
| `diff` | iki snapshot | `Vec<Change>`: yol ve eklendi, silindi ya da değişti | Hayır |
| `diff_text` | iki snapshot | `Vec<TextDiff>`: yol, tür ve bir eklentinin gösterdiği değişen satırlar ya da neden gösteremediği ([TEXT.tr.md](TEXT.tr.md)) | Hayır |
| `refs` | — | `Vec<RefInfo>`: ad ve snapshot kimliği | Hayır |
| `reflog` | ref adı | `Vec<ReflogEntry>` | Hayır |
| `resolve` | snapshot belirteci (bölüm 8.4) | snapshot kimliği | Hayır |
| `delete_ref` | ref adı, beklenen snapshot | — | Hayır |
| `repair` | — | `RepairReport`: kaldırılan kilitler, onarılan reflog'lar, yeniden indekslenen paketler, silinen bozuk indeksler | Hayır |
| `prune` | ref adı, `keep_last`, `keep_daily`, `keep_weekly`, `keep_monthly`, `keep_yearly`, deneme çalıştırması | `PruneReport`: önceki ve yeni snapshot, yeni kimlikleri ve gerekçeleriyle tutulan snapshot'lar, kaldırılan snapshot'lar ([FORMAT.tr.md](FORMAT.tr.md) §13.4); kural yoksa `invalid_input`, korunan ref için `conflict`, senkron ref'i için `unsupported` | Hayır |
| `repair_from` | uzak depo adı, isteğe bağlı hız sınırı | `CopyRepairReport`: `repair`'in yaptıkları, bozuk paketler, onlardan korunan ve uzak depodan kopyalanan nesneler, kaldırılan paketler, hiçbir kopyada sağlam olmayan nesneler ([FORMAT.tr.md](FORMAT.tr.md) §8.3) | Evet |
| `gc` | saklanacak reflog günü (varsayılan 90), deneme çalıştırması | `GcReport`: canlı nesneler; silinen nesneler, baytlar ve paketler; yazılan paketler | Evet |
| `gc_remote` | uzak depo adı, deneme çalıştırması | Uzak depo için `GcReport` ([FORMAT.tr.md](FORMAT.tr.md) §15.8); orada başka bir temizlik çalışırken `locked` | Evet |
| `keys` | — | `Vec<KeyInfo>`: kimlik, tür (`password` ya da `recovery`), eklendiği zaman, depoyu onun açıp açmadığı | Hayır |
| `add_password` | yeni parola | `AddedKey`: kimlik | Hayır |
| `add_recovery_key` | — | `AddedKey`: kimlik ve kullanıcıya bir kez gösterilen yeni kurtarma anahtarı | Hayır |
| `remove_key` | anahtar kimliği ya da ilk karakterleri | tam kimlik; son anahtar silinemez ve depo kilidi özel alınır | Hayır |
| `create_ref` | ref adı, snapshot belirteci | snapshot kimliği; ref varsa `already_exists` ([BRANCH.tr.md](BRANCH.tr.md)) | Hayır |
| `merge` | birleştirilecek snapshot belirteci, taşınacak ref (varsayılan `main`), mesaj, yazar, deneme çalışması | `MergeReport`: eylem (`up_to_date`, `fast_forward` ya da `merged`), ref'in şimdi ve önceden işaret ettiği snapshot, değişen yollar, kopyalarıyla çatışmalar, eklentilerin birleştirdiği dosyalar, eklenti uyarıları | Hayır |
| `cherry_pick` | alınacak snapshot belirteci, taşınacak ref (varsayılan `main`), mesaj, yazar, deneme çalışması | `up_to_date` ya da `picked` eylemli `MergeReport` ([BRANCH.tr.md](BRANCH.tr.md) §4) | Hayır |
| `rebase` | üstüne kurulacak snapshot belirteci (`from`), taşınacak ref (`into`), yazar, deneme çalışması | `up_to_date`, `fast_forward` ya da `rebased` eylemli `MergeReport` ([BRANCH.tr.md](BRANCH.tr.md) §5) | Hayır |
| `search` | snapshot belirteci (yoksa `main`, ya da geçmişle her ref), geçmiş, ad deseni, metin, en az boyut, en çok boyut, sonra, önce, etiketler, sınır (varsayılan 1000) | `SearchReport`: sonuçlar (yol, girdi, o sürümü tutan en yeni snapshot ve zamanı, metni içeren ilk satır), daha fazlasının olup olmadığı, aranan snapshot sayısı, metin için fazla büyük dosyalar ([SEARCH.tr.md](SEARCH.tr.md)); ölçüt yoksa `invalid_input` | Evet |
| `duplicates` | snapshot belirteci, en küçük boyut (varsayılan 0; boş dosyalar her zaman dışarıda kalır) | `Vec<Duplicate>`: bir kopyanın boyutu ve iki ya da daha fazla yol; fazla kopyaları en çok yer kaplayan gruplar önce ([SEARCH.tr.md](SEARCH.tr.md) §5) | Hayır |
| `plugins`, `add_plugin`, `remove_plugin` (serbest işlevler) | eklenti klasörü (varsayılan: kullanıcınınki), dosya ya da ad | `Vec<PluginInfo>`, `PluginAdded` (bilgi, birinin yerini alıp almadığı), `PluginInfo` ([PLUGIN.tr.md](PLUGIN.tr.md)) | Hayır |
| `conflicts`, `resolve_conflict` (serbest işlevler) | klasör; ona göre kopyanın yolu ve tutulacak sürüm (`this` ya da `copy`) | `Vec<ConflictCopy>`: yola göre sıralı olarak yol, kopya ve yoldaki dosyanın var olup olmadığı; çözülen kopyanın `ConflictCopy` değeri ([SYNC.tr.md](SYNC.tr.md) §4) | Hayır |
| `saved_searches`, `save_search`, `saved_search`, `forget_search` (serbest işlevler) | dosya (varsayılan: kullanıcınınki); bir `SavedSearch` (ad ve `search` isteği) ya da bir ad | ada göre sıralı `Vec<SavedSearch>`, hiçbir şey, `SavedSearch`, silinen `SavedSearch`; bilinmeyen bir ad için `not_found` ([SEARCH.tr.md](SEARCH.tr.md) §6) | Hayır |
| `use_plugin_dir` | klasör | —; bu tutamacın birleştirmeleri ve senkronu oradaki eklentileri kullanır | Hayır |
| `remotes` | — | `Vec<Remote>`: ad ve URL, ada göre sıralı | Hayır |
| `add_remote` | ad, URL: bir klasör yolu (mutlak ya da göreli) `sftp://[user@]host[:port]/path` ya da `s3://bucket/path?endpoint=…&region=…` | `Remote`, klasör mutlak yol olarak; SFTP ve S3 uzak depoları şifreli bir depo ister | Hayır |
| `remove_remote` | ad | —; uzak kopyanın kendisine dokunulmaz | Hayır |
| `push` | uzak depo adı, ref adı (varsayılan `main`), isteğe bağlı saniyede bayt cinsinden hız sınırı (`rate`, [SYNC.tr.md](SYNC.tr.md) §2; `pull`, `push_all`, `pull_all`, `sync` ve `clone` için de) | `TransferReport`: uzaktaki ref'in şimdi ve önceden gösterdiği snapshot'lar, kopyalanan nesneler ve saklanan baytlar; uzak depoda bu kopyada olmayan snapshot'lar varsa `conflict` | Evet |
| `pull` | uzak depo adı, ref adı (varsayılan `main`) | yerel ref için aynısı, `TransferReport`; yerel ref'te uzak depoda olmayan snapshot'lar varsa `conflict` | Evet |
| `retention` | — | `Vec<RetentionRule>`: ref deseni, gün sayısı, ne zaman eklendiği; en eskisi önce ([ARCHIVE.tr.md](ARCHIVE.tr.md)) | Hayır |
| `add_retention` | ref'ler (bir ad ya da ardından `*` gelen bir önek), gün sayısı (1 ile 36500 arası) | `RetentionRule`; asla kaldırılamaz ya da kısaltılamaz ([FORMAT.tr.md](FORMAT.tr.md) §16). Bir kuralın yasakladığı ref değişikliği `conflict` ile başarısız olur | Hayır |
| `holds` (Rust: `legal_holds`) | — | `Vec<LegalHold>`: kimlik, ref kalıbı, neden, konduğu zaman ve kaldırıldıysa kaldırmanın nedeni ve zamanı; en eskisi önce ([ARCHIVE.tr.md](ARCHIVE.tr.md) §2.1) | Hayır |
| `add_hold` (Rust: `add_legal_hold`) | ref'ler (bir ad ya da ardından `*` gelen bir önek), neden (tek satır, 1 ile 200 karakter) | `LegalHold`; kaldırılana kadar eşleştiği bir ref yalnızca ileri gider ve silinmez, gc de bütün reflog'unu tutar ([FORMAT.tr.md](FORMAT.tr.md) §17). Kilidin yasakladığı ref değişikliği `conflict` ile başarısız olur | Hayır |
| `lift_hold` (Rust: `lift_legal_hold`) | kimlik, neden | Kaldırmasıyla birlikte `LegalHold`; geri alınamaz. Bilinmeyen bir kimlik için `not_found` | Hayır |
| `report` | doğrulama, uzak depolar, eskime günü (varsayılan 7) | `ComplianceReport`: zaman, depo gerçekleri (şifreleme, anahtarlar, bozuk dosyalar), ref başına snapshot, en yeni zaman, geçmiş uzunluğu, saklama ve her uzak depodaki durum, saklama kuralları, yasal saklama kilitleri, doğrulama sonucu, uzak depolar, bulgular ([REPORTS.tr.md](REPORTS.tr.md)) | Evet |
| `push_all`, `pull_all` | uzak depo adı | `MirrorReport`: ref başına adı ve ya bir `TransferReport` ya da onu bırakan `conflict`; diğer hatalar hepsini durdurur ([REPLICATION.tr.md](REPLICATION.tr.md) §2, [FORMAT.tr.md](FORMAT.tr.md) §15.7) | Evet |
| `git_import` | Git deposu yolu, ref öneki (varsayılan `git`) | `GitImportReport`: saklanan commit ve etiketler, taşınan ref'ler (Git adı, ref, Git kimliği, snapshot, önceki), nedeniyle atlanan ref'ler, eskimiş ref'ler ([GIT.tr.md](GIT.tr.md)) | Evet |
| `git_export` | Git deposu yolu (yoksa çıplak olarak oluşturulur), ref öneki | `GitExportReport`: yazılan commit ve etiketler, güncellenen ref'ler, atlanan ref'ler, yeniden üretilemeyen Git kimlikleri | Evet |
| `sync` (`exclude`: dokunulmayacak kalıplar, [SYNC.tr.md](SYNC.tr.md) §5.1) | klasör, uzak depo adı, ref adı (varsayılan `sync/<klasör adı>`), kip (`two-way`, `send` ya da `receive`), hız sınırı, deneme çalışması | `SyncReport`: ref, eylem (`up_to_date`, `sent`, `received` ya da `merged`), snapshot, alınan yedek, ne çekildiği ve push edildiği, klasördeki değişiklikler, kopyalarıyla çatışmalar, eklentilerin birleştirdiği dosyalar, sonraki tura bırakılan yollar, uyarılar, çalışan tur sayısı ([SYNC.tr.md](SYNC.tr.md)); `send` kipinde uzak depoda bu kopyada olmayan snapshot'lar varsa `conflict` | Evet |
| `switch` | snapshot belirteci, klasör, deneme çalışması | `SwitchReport`: klasör ref'i, snapshot, klasörü önce kaydeden snapshot, klasördeki değişiklikler, geçiş sırasında değişen dosyalar ve kopyaları, dokunulmayan yollar, uyarılar ([BRANCH.tr.md](BRANCH.tr.md) §6) | Evet |
| `add_file`, `export_zip_to` (yalnızca Rust) | bir yedeğe yeni sürüm olarak eklenen dosya; bir `Write`'a zip olarak yazılan snapshot ya da içindeki yol | Dosyanın yeni yolu ve sürümü; sayılar ([SHARE.tr.md](SHARE.tr.md)) | Hayır |
| `share_plugin`, `unshare_plugin`, `shared_plugins`, `install_shared_plugin` (yalnızca Rust) | eklenti adı | `apeiron/plugins` ref'inde taşınan eklentiler ([PLUGIN.tr.md](PLUGIN.tr.md) §5) | Hayır |
| `git_serve_refs`, `git_common`, `git_pack`, `git_receive` (yalnızca Rust) | ref öneki ve Git istemcisinin istedikleri | Sunucunun HTTP üzerinden Git'e verdiği yanıtlar ([GIT.tr.md](GIT.tr.md) §7) | Hayır |

### 8.4 Bir snapshot'ı adlandırma

| Biçim | Örnek | Anlamı |
| --- | --- | --- |
| Ref adı | `main`, `backup/home` | Ref'in gösterdiği snapshot |
| Tam kimlik | 64 hex karakter | O snapshot |
| Kimlik öneki | 8 ya da daha fazla hex karakter | Kimliği bunlarla başlayan tek saklı snapshot; hiç yoksa ya da birden fazlaysa hata |
| `~N` eki | `main~2` | İlk ebeveynler üzerinden N adım geri gider |

### 8.5 İşler, ilerleme ve iptal

```rust
// API'nin biçimi, son kod değildir.
let job = repo.backup(BackupRequest::new("/home/ada/Documents", "main"))?;
while !job.is_finished() {
    let p = job.progress();           // anlık ilerlemenin bir kopyası
    println!("{} dosya, {} bayt", p.items_done, p.bytes_done);
    std::thread::sleep(Duration::from_millis(200));
}
let report = job.wait()?;            // ya da job.cancel()
```

| Kural | Açıklama |
| --- | --- |
| Async değil, iş parçacığı | Bir iş kendi iş parçacığında çalışır. Herkese açık API senkrondur; kullanmak için bir async çalışma zamanı gerekmez |
| Sorgulanan ilerleme | `job.progress()`, `Progress`'in bir kopyasını döndürür: aşama, tamamlanan öğe ve baytlar, biliniyorsa toplamlar, o anki yol. Çağıran istediği sıklıkta okur; geri çağrı yoktur, bu da C ABI'yi basit tutar |
| İptal | `job.cancel()` işin durmasını ister. İş, dosyalar ve chunk'lar arasında denetler, kısa sürede durur ve `Cancelled` hatasını döndürür. O ana kadar yazılan veri görünmez kalır; ref'ler taşınmaz |
| Tek sonuç | `job.wait()` sonucu tam bir kez döndürür; `job.wait_timeout(d)` iş hâlâ sürüyorsa erken döner |
| İşi bırakmak | Beklemeden bırakılan (drop) bir iş iptal edilir |

### 8.6 Hatalar

Her hatanın sabit numaralı bir türü, bir mesajı ve gerekiyorsa bir yolu vardır. Numaralar C ABI ve tüm SDK'larca paylaşılır ve anlamları asla değişmez.

| Kod | Tür | Anlamı |
| --- | --- | --- |
| 1 | `Io` | İşletim sistemi bir hata bildirdi (disk dolu, izin reddedildi, …) |
| 2 | `NotFound` | Depo, ref, snapshot ya da yol yok |
| 3 | `AlreadyExists` | Hedef var ve işlem üzerine yazmaz |
| 4 | `InvalidInput` | Bir istek değeri geçersiz (hatalı ref adı, hatalı snapshot belirteci, …) |
| 5 | `Ambiguous` | Bir kimlik öneki birden fazla snapshot'a uyuyor |
| 6 | `Corrupt` | Saklanan veri doğrulamadan geçemedi |
| 7 | `Locked` | Başka bir yazıcı bir kilidi tutuyor |
| 8 | `Conflict` | Bir ref eşzamanlı olarak değişti (karşılaştır ve değiştir başarısız) |
| 9 | `Cancelled` | İş iptal edildi |
| 10 | `Unsupported` | Bilinmeyen format sürümü ya da desteklenmeyen özellik |
| 11 | `Internal` | Apeiron'da bir hata; lütfen bildirin |
| 12 | `Key` | Depo şifreli ve parola ya da kurtarma anahtarı eksik veya yanlış |

Bir işlemi durdurmayan sorunlar (yedek sırasında okunamayan bir dosya, okunurken değişen bir dosya, atlanan bir aygıt dosyası) sonuçta **uyarı** olarak yer alır; her birinin bir yolu, türü ve mesajı vardır.

### 8.7 Düz veri

| Kural | Açıklama |
| --- | --- |
| Serileştirilebilir | Her istek, sonuç, ilerleme ve hata değeri JSON olarak yazılabilir (`serde`). CLI'ın `--json` çıktısı, C ABI ve ileride uzak kullanım bu biçimi kullanır |
| Kimlikler | Nesne kimlikleri JSON'da 64 karakterlik küçük harfli hex metinlerdir |
| Zamanlar | Unix saniyesi ve nanosaniye, UTC |
| Yollar ve adlar | Baytlar geçerli UTF-8 ise metin; değilse ham baytları tutan bir `{"hex": "…"}` nesnesi; böylece hiçbir ad kaybolmaz |
| Eklemeli değişiklik | Ara sürümlerde isteklere ve sonuçlara yeni isteğe bağlı alanlar eklenebilir; okuyucular bilinmeyen alanları yok sayar |

### 8.8 Sürümleme

`apeiron::API_VERSION` bu sözleşmenin sürümünü (0.1) tutar. 1.0'dan önce geriye uyumsuz değişiklikler ara sürümü yükseltir ve sürüm notlarında listelenir (bölüm 6).

## 9. Aşama 0 çekirdek işlevleri

`apeiron-core`'un bu işlevleri bölüm 8'in yapı taşlarıdır. Uygulamalar bunları doğrudan çağırmaz.

| Alan | İşlevler | Durum |
| --- | --- | --- |
| Depo | `init`, `open`, `repair`, `verify` | Tamamlandı |
| Yazma | `writer`, `put_file`, `put_tree`, `put_snapshot`, `finish` | Tamamlandı |
| Okuma | `read_file`, `read_tree`, `read_snapshot`, `history` | Tamamlandı |
| Ref'ler | `read_ref`, `update_ref`, `reflog`, `list_refs` | Tamamlandı |
| Türler | `ObjectId`, `Blob`, `Tree`, `TreeEntry`, `EntryKind`, `Snapshot`, `Time`, `ReflogEntry`, `VerifyReport`, `Error` | Tamamlandı |
