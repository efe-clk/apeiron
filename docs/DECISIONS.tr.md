> **English:** [DECISIONS.md](DECISIONS.md)

# Apeiron Mimari Karar Kayıtları

Bu belge şimdiye kadar alınan her teknik kararı ADR (architecture decision record, mimari karar kaydı) biçiminde tutar: bağlam, karar, gerekçe, değerlendirilen alternatifler ve sonuçlar. Bir karar yalnızca onun yerini alan yeni bir ADR eklenerek değiştirilir; eski kayıtlar yeniden yazılmaz.

## Dizin

| ADR | Konu | Karar | Durum |
| --- | --- | --- | --- |
| [001](#adr-001-ana-dil-olarak-rust) | Dil | Mümkün olan her yerde Rust; C yalnızca ABI için | Kabul edildi |
| [002](#adr-002-git-uyumu-için-gitoxide) | Git uyumu | gitoxide (`gix`) | Kabul edildi |
| [003](#adr-003-özet-algoritması-olarak-blake3) | Özet algoritması | BLAKE3 | Kabul edildi |
| [004](#adr-004-chunk-bölme-için-fastcdc) | Chunk bölme | FastCDC, ortalama 1 MiB | Kabul edildi |
| [005](#adr-005-eklentiler-için-wasmtime-ve-webassembly-bileşen-modeli) | Eklenti ortamı | WebAssembly bileşen modeli, Wasmtime | Kabul edildi |
| [006](#adr-006-tokio-yalnızca-çekirdeğin-dışında) | Asenkron çalışma zamanı | Tokio, yalnızca çekirdeğin dışında | Kabul edildi |
| [007](#adr-007-kişisel-yedek-ve-ev-sunucusu-ilk-hedef-kullanıcıdır) | İlk hedef kullanıcı | Kişisel yedek ve ev sunucusu | Kabul edildi |
| [008](#adr-008-çift-lisans-mit-or-apache-20) | Lisans | MIT OR Apache-2.0 | Kabul edildi |
| [009](#adr-009-nesne-kimlikleri-için-anahtarlı-blake3) | Nesne kimlikleri | Depo başına anahtarla anahtarlı BLAKE3 | Kabul edildi |
| [010](#adr-010-her-nesne-için-ayrı-dosya-yerine-paket-dosyaları) | Nesne saklama | Ayrı indeks dosyalarıyla paket dosyaları | Kabul edildi |
| [011](#adr-011-sıkıştırma-için-zstd) | Sıkıştırma | zstd | Kabul edildi |
| [012](#adr-012-yapısal-nesneler-için-kanonik-cbor) | Yapısal nesneler | Kanonik CBOR | Kabul edildi |
| [013](#adr-013-küçük-harfli-ascii-ref-adları) | Ref adları | Yalnızca küçük harfli ASCII | Kabul edildi |
| [014](#adr-014-güvensiz-sistem-çağrıları-için-küçük-bir-sistem-cratei) | `unsafe` kod | Yalnızca FFI crate'inde ve küçük `apeiron-sys` crate'inde | Kabul edildi |
| [015](#adr-015-c-abi-sınırında-json-değerler) | C ABI | Küçük ve sabit bir işlev kümesi; istekler ve sonuçlar JSON | Kabul edildi |
| [016](#adr-016-senkron-herkese-açık-api-ve-iş-parçacığında-çalışan-işler) | Uzun işlemler | Senkron API; sorgulanan ilerlemeyle iş parçacığında çalışan işler | Kabul edildi |
| [017](#adr-017-şifreleme-için-xchacha20-poly1305-ve-argon2id) | Şifreleme | Veri için XChaCha20-Poly1305, parolalar için Argon2id | Kabul edildi |
| [018](#adr-018-uzak-depo-olarak-deponun-tam-kopyası-sftp-için-russh-s3-için-object_store) | Uzak hedefler | Uzak depolar deponun tam kopyasıdır; SFTP için `russh`, S3 için `object_store` | Kabul edildi |
| [019](#adr-019-arka-plan-hizmeti-iş-dosyalı-bir-kullanıcı-sürecidir) | Arka plan hizmeti | İşletim sisteminin başlattığı, TOML iş dosyalı bir kullanıcı süreci; kaçırılan işler telafi edilir | Kabul edildi |
| [020](#adr-020-üç-yönlü-birleştirme-ve-çatışma-kopyalarıyla-bir-uzak-depo-üzerinden-senkron) | Senkron | Cihazlar bir uzak depoda buluşur; yol başına üç yönlü birleştirme; çatışan sürümler kopya olarak tutulur | Kabul edildi |
| [021](#adr-021-kesin-nesne-kimlikleriyle-kendi-crateinde-git-içe-ve-dışa-aktarma) | Git | `gix` ile `apeiron-git`; dışa aktarılan commit'ler kimliklerini korusun diye ham başlıklar `meta` içinde | Kabul edildi |
| [022](#adr-022-dallar-reftir-senkron-ve-dallar-için-tek-bir-üç-yönlü-birleştirme) | Dallar | Dallar ref'tir; senkron ve `merge` için çatışma kopyalı tek bir yol başına üç yönlü birleştirme | Kabul edildi |
| [023](#adr-023-arama-saklanan-bir-dizin-olmadan-snapshotları-okur) | Arama | Snapshot'lar doğrudan okunur, her klasör ve içerik arama başına bir kez; saklanan dizin yok | Kabul edildi |
| [024](#adr-024-python-sdksı-c-abiyi-ctypes-ile-çağırır) | Python SDK | C ABI üzerinde `ctypes` ile saf Python; her dil için tek bir uygunluk test paketi | Kabul edildi |
| [025](#adr-025-eklentiler-hiçbir-şey-verilmeden-çalışır-önce-tür-sürücüleri-gelir) | Eklentiler | İzinsiz, süre ve bellek sınırlı, kullanıcı başına bileşenler; tür sürücüleri çatışma kopyasından önce birleştirir; Rust 1.97 | Kabul edildi |
| [026](#adr-026-yerelde-sunulan-ve-yerel-bir-pencerede-gösterilen-tek-bir-web-sayfaları-takımı) | Masaüstü arayüzü | Derleme adımı olmayan HTML sayfaları, `127.0.0.1` üzerinde jetonlu bir JSON API, bir `wry` penceresi | Kabul edildi |
| [027](#adr-027-web-arayüzü-tek-bir-depoyu-tls-üzerinden-parolayla-oturum-açana-sunar) | Web arayüzü | Başlarken sabitlenen tek depo; çerezli oturumla parola girişi; varsayılan olarak kendinden imzalı sertifikayla `rustls`; izinli işlemler ve klasörler | Kabul edildi |
| [028](#adr-028-üç-rollü-kullanıcılar-ref-adıyla-yedekler-ve-üst-sürümler-yoluyla-görünürlük) | Kullanıcılar ve izinler | `admin`, `member`, `reader`; önekli ref adları olarak yedekler; kullanıcının ref'lerinden ulaşılabilen snapshot görünür | Kabul edildi |
| [029](#adr-029-denetim-kaydı-her-şeyden-önce-yazılan-özet-zincirli-bir-json-satırları-dosyasıdır) | Denetim kaydı | SHA-256 zincirli JSON satırları; bir kilit altında eklenir ve istek sürmeden önce diske aktarılır; kaydedilemeyen istek başarısız olur | Kabul edildi |
| [030](#adr-030-paylaşım-yalnızca-özetleri-tutulan-sabit-bir-sürüme-süreli-bağlantılarladır) | Paylaşım | Bağlantıda rastgele bir gizli kısım, yalnızca SHA-256 özeti tutulur; sabit bir sürüm ve yol; 1 ile 30 gün; sahibi artık paylaşamadığında biter | Kabul edildi |
| [031](#adr-031-merkezi-kimlik-kullanıcıları-hâlâ-webjson-içinde-listelenen-openid-connecttir) | Merkezi kimlik | PKCE'li OpenID Connect kod akışı; sağlayıcının anahtarlarıyla denetlenen RS256/ES256 belirteçleri; bir alan `web.json` içinde listelenen bir kullanıcıyı adlandırır; parolalar mümkün kalır | Kabul edildi |
| [032](#adr-032-kiracılar-ana-makineyle-ayrılır-her-birinin-kendi-deposu-ve-dosyaları-vardır) | Çok kiracılı kullanım | Tek süreç, tek port; `Host` başlığı, kendi deposu, kullanıcıları, denetim kaydı, paylaşım bağlantıları ve oturumları olan bir kiracı seçer; bilinmeyen ana makineler hiçbir şey almaz | Kabul edildi |
| [033](#adr-033-replikasyon-her-refi-uzak-bir-kopyaya-push-etmektir-görevi-devralmak-adminin-kararıdır) | Replikasyon ve yüksek erişilebilirlik | Her ref her yedekten sonra ve değişince her 30 saniyede replikalara push edilir; yedek sunucu bir replikayı yedek almadan sunar; `/api/health`; görev devri admin tarafından | Kabul edildi |

ADR 001–008, Aşama 0 başlamadan önce alındı ([VISION.tr.md](VISION.tr.md)). ADR 009–013, depolama formatıyla birlikte 2026-10-27'de onaylandı ([FORMAT.tr.md](FORMAT.tr.md)). ADR-014, 2026-10-28'de onaylandı.

## ADR-001: Ana dil olarak Rust

**Durum:** Kabul edildi

**Bağlam.** Apeiron başkalarının verisini işler, Raspberry Pi 4 kadar küçük cihazlarda çalışır ve başka dillerle yazılmış uygulamalara gömülebilmelidir. Depolama kodundaki bellek güvenliği hataları veriyi sessizce bozabilir.

**Karar.** Apeiron mümkün olan her yerde Rust ile yazılır. C yalnızca dışa açılan ABI katmanında kullanılır. `unsafe` yalnızca FFI crate'inde serbesttir.

**Gerekçe.** Rust derlenir ve hızlıdır, bellek hatalarının bütün bir sınıfını derleme anında engeller, çöp toplayıcı duraklamaları yoktur ve diğer dillerdeki SDK'lar için bir C ABI açabilir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| C veya C++ | Bellek güvenliği hataları veri bütünlüğü için doğrudan risk |
| Go | Çöp toplayıcı ve çalışma zamanı, C ABI üzerinden gömmeyi zorlaştırır ve küçük cihazlarda bellek kullanımını öngörülemez kılar |
| Zig | Umut verici, ama uzun ömürlü bir depolama formatı için dil ve ekosistem henüz yeterince kararlı değil |

**Sonuçlar.** Katkı verenlerin Rust bilmesi gerekir. Olgun bir Rust alternatifi olmayan yerlerde bazı C kütüphaneleri bağlayıcılar (binding) üzerinden kullanılabilir (bkz. ADR-011). `unsafe` gerektiren platform çağrıları `apeiron-sys` crate'inde yer alır (ADR-014).

## ADR-002: Git uyumu için gitoxide

**Durum:** Kabul edildi

**Bağlam.** Yazılımcılar Git depolarını Apeiron'a taşıyıp geri alabilmelidir. Git içe/dışa aktarma zorunludur; Git ağ protokolü (SSH, smart HTTP) hedeftir ama şart değildir.

**Karar.** Git uyumu için gitoxide (`gix`) kullanılır. Git protokolü sıfırdan yazılmaz. Git desteği çekirdekte değil, bir eklentide ya da hizmette yer alır.

**Gerekçe.** gitoxide saf Rust ile yazılmış bir Git uygulamasıdır; ADR-001'e uyar ve C bağımlılığını önler.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| libgit2 (`git2` crate'i) | Bir C kütüphanesi; ADR-001 ile çelişir |
| `git` komutunu çağırmak | Git'in kurulu olmasını ve metin çıktısının ayrıştırılmasını gerektirir; kırılgan |
| Kendi Git uygulamamızı yazmak | Büyük ve hataya açık; projenin amacı bu değil |

**Sonuçlar.** Git özelliklerinin kapsamı, özellikle ağ protokolünde, gitoxide'ın ilerlemesine bağlıdır. Bu iş Aşama 3'te planlandı.

## ADR-003: Özet algoritması olarak BLAKE3

**Durum:** Kabul edildi

**Bağlam.** Her nesne kriptografik bir özetle (hash) adreslenir ve doğrulanır. Özetleme, Raspberry Pi dahil, yazılan ve okunan her bayt üzerinde çalışır.

**Karar.** Tüm özetleme işlerinde BLAKE3 (`blake3` crate'i) kullanılır.

**Gerekçe.** BLAKE3 kriptografik bir özettir; paralel ve çok hızlıdır, ARM üzerinde de verimlidir ve resmi bir Rust uygulaması vardır. Anahtarlı bir modu da vardır (ADR-009 bunu kullanır).

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| SHA-256 | Güvenli, ama donanım hızlandırması olmadan belirgin şekilde yavaş |
| SHA-1 (Git'teki gibi) | Çakışma direnci kırılmış durumda |
| xxHash ve diğer kriptografik olmayan özetler | Hızlı, ama kasıtlı çakışmalara karşı koruma sağlamaz |

**Sonuçlar.** Git nesnelerinin kimlikleri (SHA-1/SHA-256) Apeiron kimliği olarak yeniden kullanılmaz; Git içe aktarma bunları dönüştürür.

## ADR-004: Chunk bölme için FastCDC

**Durum:** Kabul edildi

**Bağlam.** Dosyalar, küçük bir değişiklik yalnızca birkaç chunk'ı yeniden yazdıracak şekilde bölünmelidir. Tekilleştirme, artımlı yedek ve devam ettirilebilir aktarım buna dayanır.

**Karar.** FastCDC (2020 sürümü): ortalama chunk boyutu 1 MiB, en küçük 256 KiB, en büyük 4 MiB. Ortalama 64 KiB ile 4 MiB arasında ayarlanabilir.

**Gerekçe.** İçerik tanımlı bölme (content-defined chunking), veri eklenip çıkarıldığında chunk sınırlarını sabit tutar. FastCDC bu türün en hızlı algoritmalarındandır ve bir Rust uygulaması (`fastcdc`) vardır. 1 MiB, tekilleştirme ile nesne sayısı arasında denge kurar.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Sabit boyutlu chunk'lar | Araya eklenen tek bir bayt sonraki bütün chunk'ları kaydırır; tekilleştirme çöker |
| Rabin parmak izi | Çalışır, ama FastCDC'den yavaş |
| Buzhash (Borg'daki gibi) | Benzer sonuç; FastCDC daha hızlı ve chunk boyutlarını normalleştirir |

**Sonuçlar.** Chunk bölme ayarları `config` içinde saklanır ama yalnızca yazarken gerekir; okuyucular blob'lardaki chunk listelerini kullanır. Ayarları değiştirmek eski veriyi okunur bırakır, yalnızca eski ve yeni veri arasındaki tekilleştirmeyi azaltır.

## ADR-005: Eklentiler için Wasmtime ve WebAssembly bileşen modeli

**Durum:** Kabul edildi

**Bağlam.** Alana özgü davranış (diff, önizleme, depolama hedefleri, içe aktarıcılar) çekirdeğe dokunmadan eklenebilmeli ve hatalı bir eklenti veriyi tehlikeye atmamalıdır.

**Karar.** Eklentiler, Wasmtime'ın çalıştırdığı WebAssembly bileşenleridir.

**Gerekçe.** WebAssembly, sandbox'ı ve yetenek tabanlı izin modelini hazır getirir. Wasmtime Rust ile yazılmıştır. Bileşen modeli, eklentilerin WebAssembly'ye derlenen her dilde yazılabilmesini sağlar.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Yerel paylaşımlı kütüphaneler | Sandbox yok; bir çökme ana sistemi de düşürür |
| Gömülü betik dili (Lua, Python) | Eklenti yazarlarını tek dile bağlar; yalıtım daha zayıf |
| IPC ile ayrı süreçler | Güçlü yalıtım, ama küçük cihazlarda ağır ve dağıtımı zor |
| Diğer WASM çalışma zamanları | Bileşen modeli desteği en eksiksiz olan ve Rust ile yazılan Wasmtime |

**Sonuçlar.** Eklenti API'sinin kendi sürümlemesi olmalıdır. Eklentiler yerel koda göre küçük bir performans bedeli öder. Bu iş Aşama 3'te planlandı.

## ADR-006: Tokio yalnızca çekirdeğin dışında

**Durum:** Kabul edildi

**Bağlam.** Ağ, senkron ve arka plan hizmeti doğası gereği asenkrondur. Çekirdek ise gömmesi ve test etmesi kolay kalmalıdır.

**Karar.** Asenkron çalışma zamanı Tokio'dur ve yalnızca ağ, senkron ve arka plan hizmeti katmanlarında kullanılır. Çekirdek senkrondur.

**Gerekçe.** Tokio, Rust'ta ağ işleri için fiili standarttır. Çekirdeğin senkron kalması, uygulamaların onu bir asenkron çalışma zamanı benimsemeden kullanabilmesini sağlar ve kesinti güvenliği mantığını kolay izlenir tutar.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| async-std | Artık geliştirilmiyor |
| smol | Gereken ağ kütüphaneleri için ekosistemi küçük |
| Asenkron çekirdek | Çekirdeği gömen herkesi asenkron çalışma zamanına zorlar; hata enjeksiyonu testlerini zorlaştırır |

**Sonuçlar.** Hizmet katmanı, bloklayan çekirdek çağrılarını asenkron koda köprülemelidir (örneğin `spawn_blocking` ile).

## ADR-007: Kişisel yedek ve ev sunucusu ilk hedef kullanıcıdır

**Durum:** Kabul edildi

**Bağlam.** Apeiron altı kullanıcı profilini hedefler. Hepsi için aynı anda geliştirmek, kullanılabilir bir ürünü geciktirir.

**Karar.** İlk hedef kullanıcı kişisel yedek ve ev sunucusudur. Geliştirici özellikleri Aşama 3'te gelir.

**Gerekçe.** En dar gerçek kullanım alanıdır ve doğrudan Aşama 1'in çıkış kriteriyle ölçülür: kendi verini bir ay boyunca yalnızca Apeiron ile yedekleyip geri yükleyebilmek.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Önce yazılımcılar | Git ile doğrudan rekabet eder; önce dal, birleştirme ve Git uyumu gerekir |
| Önce yaratıcı meslekler | Type driver'lar (ses, görsel), dolayısıyla önce eklenti sistemi gerekir |
| Önce kurumlar | Önce kimlik, izin ve denetim özellikleri gerekir |

**Sonuçlar.** Aşama 1–2 yedek, geri yükleme, doğrulama ve senkronu öne alır. Geliştiriciye yönelik özellikler Aşama 3'e kadar bekler.

## ADR-008: Çift lisans: MIT OR Apache-2.0

**Durum:** Kabul edildi

**Bağlam.** Apeiron, kütüphanesi ve SDK'ları aracılığıyla başka uygulamalara gömülmek üzere tasarlanır.

**Karar.** Apeiron, kullanıcının seçimine bağlı olarak MIT veya Apache-2.0 lisansıyla sunulur.

**Gerekçe.** Rust ekosisteminin standardıdır ve SDK'nın başka uygulamalara gömülmesini kolaylaştırır. Apache-2.0 ayrıca açık bir patent izni verir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| GPL-3.0 | GPL olmayan uygulamalara gömülmeyi engeller |
| MPL-2.0 | Kullanılabilir, ama Rust ekosisteminde daha az yaygın |
| Yalnızca MIT | Açık bir patent izni yok |
| Yalnızca Apache-2.0 | GPL-2.0 projeleriyle uyumsuz |

**Sonuçlar.** Katkılar aynı çift lisansla kabul edilir ([CONTRIBUTING.tr.md](../CONTRIBUTING.tr.md)). Bağımlılıkların lisansları uyumlu olmalıdır.

## ADR-009: Nesne kimlikleri için anahtarlı BLAKE3

**Durum:** Kabul edildi (2026-10-27)

**Bağlam.** Aşama 2'de uzak hedefler şifreli veri saklayacak. Nesne kimlikleri düz içerik özeti olsaydı, uzak hedef bilinen bir dosyanın özetini hesaplayarak o dosyanın depoda olup olmadığını anlayabilirdi.

**Karar.** Nesne kimlikleri, depo başına rastgele 32 baytlık bir anahtarla `BLAKE3_keyed(id_key, tür_baytı ‖ içerik)` olarak hesaplanır. Paket ve indeks dosyalarının adları düz BLAKE3'tür ([FORMAT.tr.md](FORMAT.tr.md) §4).

**Gerekçe.** Anahtarlı kimlik, anahtarı olmayan birine içerik hakkında hiçbir şey söylemez; düz dosya adları ise anahtarı bilmeyen bir sunucunun da bozulmuş dosyaları fark etmesini sağlar.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| İçeriğin düz BLAKE3 özeti | Hangi bilinen dosyaların saklandığını sızdırır |
| Yalnızca şifreleme açıkken anahtarlı | İki ayrı kimlik şeması; şifrelemeyi değiştirmek her şeyin yeniden özetlenmesini gerektirir |

**Sonuçlar.** Birbirinden bağımsız oluşturulan iki depo arasında tekilleştirme olmaz. Aynı deponun kopyaları (senkron, yedek hedefleri) anahtarı paylaşır.

## ADR-010: Her nesne için ayrı dosya yerine paket dosyaları

**Durum:** Kabul edildi (2026-10-27)

**Bağlam.** 100 bin dosyalık bir çalışma alanı en az o kadar nesne üretir. Her birini ayrı dosya olarak saklamak küçük cihazlarda yavaş, istek başına ücret alan uzak hedeflerde pahalıdır.

**Karar.** Nesneler paket dosyalarında (önerilen 16–64 MiB) saklanır; paketlerden yeniden üretilebilen ayrı indeks dosyaları vardır ([FORMAT.tr.md](FORMAT.tr.md) §8).

**Gerekçe.** Daha az ve daha büyük dosya, daha az dosya sistemi işlemi ve daha az uzak istek demektir. İndeksler yalnızca önbellek olduğu için kaybolmaları hiçbir zaman veri kaybettirmez.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Her nesne için bir dosya (gevşek nesneler) | Çok fazla küçük dosya; uzak hedeflerde yavaş ve pahalı |
| Gömülü bir veritabanı (ör. SQLite) | Apeiron olmadan okunması zor; açık format ilkesiyle çelişir |

**Sonuçlar.** Nesne silmek paketlerin yeniden yazılmasını gerektirir (çöp toplama, Aşama 1).

## ADR-011: Sıkıştırma için zstd

**Durum:** Kabul edildi (2026-10-27)

**Bağlam.** Pek çok nesne iyi sıkışır; sıkıştırma disk alanından ve aktarım süresinden kazandırır.

**Karar.** Paketlerdeki nesneler, boyutu küçülüyorsa zstd (düzey 3) ile sıkıştırılır; küçülmüyorsa sıkıştırılmadan saklanır ([FORMAT.tr.md](FORMAT.tr.md) §8.1).

**Gerekçe.** zstd hız ile sıkıştırma oranı arasında çok iyi bir denge sunar ve yaygındır; bağımsız okuyucular da açabilir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| LZ4 | Daha hızlı, ama sıkıştırma oranı belirgin şekilde düşük |
| Saf Rust zstd uygulaması | Yazma tarafı henüz yeterince olgun değil |
| Sıkıştırmasız | Disk alanını ve bant genişliğini boşa harcar |

**Sonuçlar.** `zstd` crate'i C zstd kütüphanesine bağlanır; bu, "mümkün olan her yerde Rust" kuralına (ADR-001) kabul edilmiş bir istisnadır. Derleme için bir C derleyicisi gerekir.

## ADR-012: Yapısal nesneler için kanonik CBOR

**Durum:** Kabul edildi (2026-10-27)

**Bağlam.** Blob, tree ve snapshot özetlenir; bu yüzden aynı nesne her zaman tam olarak aynı baytlara kodlanmalı ve bağımsız okuyucular onları çözebilmelidir.

**Karar.** Yapısal nesneler, temel belirlenimci kodlama (core deterministic encoding) ve tamsayı harita anahtarlarıyla CBOR (RFC 8949) kullanır ([FORMAT.tr.md](FORMAT.tr.md) §6).

**Gerekçe.** CBOR, birçok dilde uygulaması olan bir standarttır; belirlenimci kodlama özetlerin sabit kalmasını garanti eder; tamsayı anahtarlar nesneleri küçük tutar.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Özel bir ikili format | Her bağımsız okuyucunun özel bir ayrıştırıcı yazması gerekir |
| JSON | Bayt dizisi yok; kanonik JSON yaygın biçimde standartlaşmamış |
| Protocol Buffers | Kodlaması tasarım gereği kanonik değil |

**Sonuçlar.** Yazıcı kanonik kuralları uygulamalıdır; okuyucular kanonik olmayan nesneleri yakalamak için yeniden kodlayıp karşılaştırır. Kütüphane `minicbor`'dır (2026-10-28'de seçildi): yazılan baytlar üzerinde tam denetim verir ve etkin olarak bakımı yapılmaktadır.

## ADR-013: Küçük harfli ASCII ref adları

**Durum:** Kabul edildi (2026-10-27)

**Bağlam.** Ref'ler dosya olarak saklanır. Büyük/küçük harf ayırmayan dosya sistemlerinde (Windows, macOS) `Main` ile `main` çakışır.

**Karar.** Ref adlarında yalnızca `a-z`, `0-9`, `.`, `_`, `-` ve ayırıcı olarak `/` kullanılabilir ([FORMAT.tr.md](FORMAT.tr.md) §9.3).

**Gerekçe.** Her platformda çakışmaları ve kodlama sorunlarını ortadan kaldırır.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Herhangi bir Unicode ad | Büyük/küçük harf eşleme ve Unicode normalleştirmesi platformdan platforma değişir |
| Adları dosya adına kodlamak (ör. hex) | Ref dosyaları artık insan tarafından okunamaz |

**Sonuçlar.** Serbest görünen adlar (Türkçe karakterler dahil) hizmet katmanında tutulur ve ref adlarına eşlenir.

## ADR-014: Güvensiz sistem çağrıları için küçük bir sistem crate'i

**Durum:** Kabul edildi (2026-10-28)

**Bağlam.** Windows'ta bir yeniden adlandırma, çağrı dönmeden diske ancak `MoveFileExW(MOVEFILE_WRITE_THROUGH)` ile ulaşır ([FORMAT.tr.md](FORMAT.tr.md) §9.1). Standart kütüphane bunu sunmaz ve Windows API'sini çağırmak `unsafe` kod gerektirir. Şimdiye kadar `unsafe` yalnızca çekirdeğin üstünde duran ve çekirdeğin kullanamadığı FFI (C ABI) crate'inde serbestti.

**Karar.** `unsafe` gerektiren işletim sistemi çağrıları, çekirdeğin bağımlı olduğu ayrı ve küçük bir `apeiron-sys` crate'inde yer alır. Microsoft'un resmi `windows-sys` bağlarını kullanır. Her `unsafe` bloğunun neden güvenli olduğunu açıklayan bir yorumu, her işlevin bir testi vardır. FFI crate'i dışındaki tüm diğer crate'ler `#![forbid(unsafe_code)]` kuralını korur.

**Gerekçe.** Güvensiz kod, incelenmesi kolay tek bir yerde kalır ve katman kuralı (her katman yalnızca altındakileri kullanır) korunur.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Windows'ta write-through'u ertelemek | NTFS yarım durumları zaten engeller, ama elektrik kesintisinden sonra son işlem geri alınmış gibi görünebilir |
| Çağrıyı C ABI crate'ine koymak | Çekirdek üstündeki bir crate'e bağımlı olur; katman kuralı bozulur |
| Üçüncü taraf bir sarmalayıcı crate | Bu çağrıyı sunan, bakımı yapılan ve yaygın kullanılan bir crate bulunamadı |

**Sonuçlar.** `apeiron-sys` küçük kalmalıdır. Oraya yeni bir işlev yalnızca güvenli bir alternatif yoksa eklenir.

## ADR-015: C ABI sınırında JSON değerler

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Her SDK C ABI üzerine kurulur ([SDK.tr.md](SDK.tr.md)). Her işlem için bir C işlevi ve her istek ile sonuç için C yapıları ABI'yi büyütür; her yeni alan onu bozar.

**Karar.** C ABI küçük ve sabit bir işlev kümesinden oluşur (aç, çağır, başlat, ilerleme, iptal, bekle, bırak). İşlem bir metinle adlandırılır; istekler, sonuçlar, ilerleme ve hatalar [API.tr.md](API.tr.md) §8.7 biçiminde UTF-8 JSON'dır ve Rust türlerinden üretilen JSON Schema'larla tanımlanır.

**Gerekçe.** Yeni işlemler ve yeni isteğe bağlı alanlar ABI değişikliği gerektirmez. Her dil JSON okuyabilir ve aynı değerler CLI'ın `--json` çıktısına ve ileride uzak kullanıma da hizmet eder. Küçük değerleri JSON'a çevirmenin maliyeti disk ve ağ işinin yanında yok sayılabilir; dosya içeriği hiçbir zaman JSON'dan geçmez.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| C yapıları ve işlem başına bir işlev | Büyük ABI; bir alan eklemek ikili uyumluluğu bozar |
| CBOR ya da Protocol Buffers gibi ikili bir kodlama | Daha hızlı, ama hata ayıklaması zor ve her SDK'da ek bir kütüphane gerektirir |
| C ABI olmadan dile özgü bağlar (PyO3, …) | Her dilin kendi yapıştırıcı kodu olur; "her yerde aynı sonuç" hedefine ters düşer |

**Sonuçlar.** SDK'lar tipli sınıflarını şemalardan üretir. İsteklerdeki tür hataları çalışma anında kütüphane tarafından bulunur ve `InvalidInput` olarak bildirilir.

## ADR-016: Senkron herkese açık API ve iş parçacığında çalışan işler

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Yedekleme ve geri yükleme dakikalar ya da saatler sürer. Çağıranların ilerlemeye ve iptale ihtiyacı vardır; çekirdek ise senkrondur (ADR-006).

**Karar.** Herkese açık API senkrondur. Uzun bir işlem kendi iş parçacığında çalışır ve bir iş olarak döndürülür; çağıran ilerlemeyi sorgulayarak okur, işi `cancel` ile durdurur ve sonucu `wait` ile alır ([API.tr.md](API.tr.md) §8.5).

**Gerekçe.** Çağıranın bir async çalışma zamanına ihtiyacı olmaz; sorgulama C ABI'yi geri çağrısız geçer, böylece Apeiron'un iş parçacıklarında yabancı kod çalışmaz; iptal, durmanın her zaman güvenli olduğu dosyalar ve chunk'lar arasında denetlenir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Async bir API (Tokio) | Her çağırana ve her SDK'ya bir çalışma zamanı dayatır |
| İlerleme geri çağrıları | Apeiron'un iş parçacıklarından yabancı kod çağırmak C ABI üzerinden kırılgandır (iş parçacıkları, istisnalar, çöp toplayıcılar) |
| Yalnızca bloklayan çağrılar | İlerleme de iptal de olmaz |

**Sonuçlar.** SDK'lar sorgulamanın üzerine kendi dillerinde geri çağrı ya da yineleyici ekleyebilir.

## ADR-017: Şifreleme için XChaCha20-Poly1305 ve Argon2id

**Durum:** Kabul edildi (2026-10-28)

**Bağlam.** Aşama 2, depoları güvenilmeyen disklerde ve sunucularda saklar ([FORMAT.tr.md](FORMAT.tr.md) §14). Şifre, birbirinden bağımsız birkaç yazıcının rastgele seçtiği nonce'larla güvenli olmalı, bir Raspberry Pi 4'te hızlı çalışmalı ve bakımı yapılan saf Rust bir kütüphane olarak bulunmalıdır. Parolalar için yavaş ve çok bellek isteyen bir türetme gerekir.

**Karar.** Veri, RustCrypto `chacha20poly1305` crate'i ile XChaCha20-Poly1305 kullanılarak şifrelenir (24 baytlık rastgele nonce'lar, 16 baytlık etiketler). Parola anahtarları RustCrypto `argon2` crate'i ile Argon2id kullanılarak türetilir (varsayılan 64 MiB, 3 tur, 1 şerit). Kurtarma anahtarları rastgeledir ve BLAKE3 anahtar türetmesini kullanır.

**Gerekçe.** 24 baytlık bir nonce, yazıcılar arasında paylaşılan bir sayaç olmadan rastgele nonce'ları güvenli kılar; ChaCha20, Raspberry Pi 4'te bulunmayan AES donanımı olmadan da hızlıdır; Argon2id parola özetleme için güncel öneridir; iki kütüphane de yaygın kullanılır, saf Rust'tır ve Rust 1.85'i destekler.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| AES-256-GCM | 12 baytlık nonce'ları büyük ölçekte rastgele seçmek güvenli değildir ve AES donanımı olmadan yavaştır |
| AES-256-GCM-SIV | Yanlış kullanıma dayanıklıdır, ama AES donanımı olmadan yavaştır ve Rust'ta daha az yaygındır |
| Parolalar için scrypt ya da PBKDF2 | GPU saldırılarına karşı Argon2id'den zayıftır |
| Paket dosyalarının tamamını tek bir akış olarak şifrelemek | Tek bir nesneyi okumak tüm paketin şifresini çözmeyi gerektirir |

**Sonuçlar.** Şifreli paketler biraz büyüktür (nesne ve tablo başına 40 bayt). Ref adları görünür kalır (§14.5). Kriptografi için RustCrypto dışındaki kütüphaneler kullanılmaz.

## ADR-018: Uzak depo olarak deponun tam kopyası; SFTP için `russh`, S3 için `object_store`

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Aşama 2, yedekleri harici bir diske, bir ev sunucusuna ve bulut depolamaya gönderir ([FORMAT.tr.md](FORMAT.tr.md) §15). Uzak depolama yavaştır ve her an kesilebilir; bulut depolamada dosya yeniden adlandırma ve kilit yoktur. Windows, macOS ve Linux'un hepsi desteklenir ve çekirdek senkron kalır (ADR-006).

**Karar.** Uzak depo, aynı anahtarlara sahip, deponun eksiksiz bir kopyasıdır; push ve pull yalnızca bir ref'in eksik olan nesnelerini saklandıkları (hâlâ şifreli) baytlarla kopyalar ve ref'leri yalnızca ileri taşır. Her uzak depo aynı küçük işlem kümesini sunar (okuma, listeleme, yeni bir dosyayı atomik yazma, bir ref'i koşullu değiştirme, silme, yeni bir kopyanın klasörlerini oluşturma). Bu `Store` arayüzü ve klasör deposu `apeiron-core` içindedir; böylece push ve pull her uzak depo türü için bir kez yazılır. Klasörler Apeiron'un kendi kalıcı yazma yolunu kullanır. SFTP `russh` ve `russh-sftp` kullanır. S3 ve S3 uyumlu depolama `object_store` kullanır. Bu kütüphaneler, yeni `apeiron-remote` crate'inin içinde, bekleyen (blocking) bir arayüzün arkasında Tokio üzerinde çalışır.

**Gerekçe.** Tam bir kopya, asıl bilgisayar olmadan geri yüklenebilir ve yeni dosya türleri gerektirmez. Saklanan baytları kopyalamak, uzak deponun hiçbir zaman parolaya ihtiyaç duymaması ve verinin yolda hiç çözülmemesi demektir. `russh` saf Rust'tır ve sistemdeki `ssh` programını çalıştıran kütüphanelerin aksine Windows'ta çalışır. `object_store` Apache Arrow projesi tarafından bakılır ve güvenli bir ref güncellemesinin gerektirdiği koşullu yazmayı S3'te destekler. İkisi de Rust 1.85'i destekler.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Her yedek için tek bir şifreli arşiv dosyası | Her yedek her şeyi yeniden yükler; tekrar eden veri ayıklanmaz |
| SFTP için `ssh2` (libssh2) | C kütüphanesidir; ADR-001 Rust'ı tercih eder |
| `openssh-sftp-client` | Sistemdeki `ssh` programını çalıştırır; Windows'ta çalışmaz |
| Her uzak depo için `opendal` | Onun SFTP desteği de sistemdeki `ssh` programını çalıştırır |
| Koşullu yazma yerine S3'te kilit dosyaları | S3'te koşullu yazma olmadan atomik "yoksa oluştur" yoktur |

**Sonuçlar.** `russh`, Rust 1.85'i destekleyen en yeni sürüm olan 0.63'e, saf Rust `ring` şifreleme altyapısıyla sabitlenir; `object_store`, HTTPS için `aws-lc-rs` getirir. Koşullu yazma desteği olmayan S3 uyumlu hizmetler reddedilir. SFTP, sunucunun `~/.ssh/known_hosts` içinde olmasını ister. SFTP ve S3 kopyalarının reflog'u yoktur. Bir uzak kopyanın çöp toplaması sonraki bir iştir; bir klasör kopyası, bir depo olarak açılarak bugün de toplanabilir.

## ADR-019: Arka plan hizmeti iş dosyalı bir kullanıcı sürecidir

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Aşama 2, zamanlanmış yedeklere ve düzenli bir geri yükleme testine ihtiyaç duyar ([DAEMON.tr.md](DAEMON.tr.md)). İlk kullanıcılar, Linux, macOS ve Windows'ta, çoğu zaman uyuyan ya da kapatılan bir dizüstü olan kişisel bir bilgisayarı yedekler. Şifreli depolar, başında biri olmadan parolalarına ihtiyaç duyar.

**Karar.** `apeiron daemon run`, oturum açmış kullanıcının sıradan bir sürecidir ve oturum açılınca işletim sisteminin kendi yoluyla başlatılır (systemd kullanıcı birimi, launchd ajanı, Görev Zamanlayıcı). İşler, kullanıcının yapılandırma klasöründeki bir TOML dosyasında listelenir; kaçırılan bir iş ilk fırsatta bir kez çalışır. Her işin durumu bir JSON dosyasında tutulur ve daemon, yalnızca herkese açık API'yi kullanan ayrı bir crate'tir: `apeiron-daemon`. Parolalar, CLI'daki gibi parola dosyalarından gelir.

**Gerekçe.** Bir kullanıcı süreci, kullanıcının dosyalarını ve anahtarlarını yönetici yetkisi olmadan okuyabilir ve aynı kod üç sistemde de çalışır; yalnızca başlatma dosyası değişir. Kaçırılan çalışmaları telafi etmek, bir zamanlamayı dizüstünde çalışır kılan şeydir. Düz metin bir iş dosyası kullanıcı tarafından okunabilir, düzenlenebilir ve yedeklenebilir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Yönetici olarak çalışan sistem çapında bir hizmet | Yönetici yetkisi ve her kullanıcının dosyalarına erişim ister; kişisel bir yedeğin ihtiyacından fazladır |
| Tek tek CLI komutları çalıştıran `cron`, launchd ya da Görev Zamanlayıcı kayıtları | Her sistemde farklıdır; `cron`, bilgisayar kapalıyken kaçırılan çalışmaları telafi etmez; `status` için ortak bir durum yoktur |
| İşleri deponun içinde tutmak | Bir iş birden fazla depoya ve uzak depoya yayılabilir; depo tek bir bilgisayarın klasörlerine bağlı olmamalıdır |
| cron biçiminde bir zamanlama (`30 2 * * *`) | İlk kullanıcılar için okunması zordur; kişisel yedekler için `every` ve `at` yeterlidir |

**Sonuçlar.** İşler yalnızca kullanıcı oturum açmışken çalışır. Bir bilgisayarın iki kullanıcısı kendi daemon'larını çalıştırır. Oturum açmış kullanıcısı olmayan bir sunucu, `apeiron daemon run` komutunu elle bir sistem hizmeti olarak çalıştırır; rehber bunun nasıl yapılacağını gösterir.

## ADR-020: Üç yönlü birleştirme ve çatışma kopyalarıyla bir uzak depo üzerinden senkron

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Aşama 2, iki cihaz ve bir uzak sunucu arasında bir ay sorunsuz senkronla biter ([SYNC.tr.md](SYNC.tr.md)). Cihazlar çoğu zaman kapalı olan dizüstü ve masaüstü bilgisayarlardır; uzak depo bir disk, bir SFTP sunucusu ya da S3'tür (ADR-018). Senkron hiçbir değişikliği sessizce kaybetmemeli ve bir silmenin tek kopyayı yok etmesinin yolu olmamalıdır (VISION.tr.md).

**Karar.** Bir senkron turu klasörü yedekler, uzaktaki ref'i pull eder ve ya push eder, ya uzaktaki snapshot'ı klasöre uygular, ya da ikisini son ortak snapshot'ı taban alarak birleştirir. Birleştirme yol başına çalışır; iki taraf da bir dosyayı farklı değiştirdiyse yerel sürüm kalır, öteki cihazının ve zamanının adını taşıyan bir çatışma kopyası olarak eklenir. Klasör yalnızca, turun başında alınan snapshot'a hâlâ eşit olduğu yerlerde değiştirilir. Yeni bir cihaz, uzak deponun config'ini ve anahtarlarını kopyalayan `clone` ile başlar.

**Gerekçe.** Her şey snapshot'lardan yapıldığı için her tur geri yüklenebilir bir geçmiştir ve push, iki cihazın birbirinin ref'inin üzerine yazamamasını zaten güvence altına alır. Bir uzak depoda buluşmak, cihazlar hiç birlikte çevrim içi olmasa da çalışır. Çatışma kopyaları insanların Dropbox ve Syncthing'den bildiği şeydir ve çözmek için bir araç gerektirmez; bir çözüm aracı masaüstü arayüzüyle gelebilir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Cihazlar arasında eşler arası senkron | İki cihazın aynı anda çevrim içi olması gerekir; keşif ve NAT geçişi gerektirir; uzak depo zaten var |
| Son yazan kazanır | Öteki cihazın değişikliğini sessizce kaybeder |
| Kullanıcı karar verene kadar turu bir çatışmada durdurmak | Senkron diğer her dosya için durur; kullanıcı günlerce bakmayabilir |
| Snapshot'lar yerine dosya başına sürüm vektörleri | Snapshot'ların yanında ikinci bir model; yayılan silmelere karşı koruyan geçmişi kaybeder |

**Sonuçlar.** Çatışma kopyaları klasörde belirir ve elle temizlenmelidir. Bir tur sırasında değişen bir dosya sonraki turu bekler. Depo geçmişi, değişiklik içeren her tur için bir snapshot büyür; ileride saklama kurallarıyla çöp toplama bunu dengeler.

## ADR-021: Kesin nesne kimlikleriyle, kendi crate'inde Git içe ve dışa aktarma

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Aşama 3, gerçek bir Git deposu Apeiron'a taşınıp geri alınabildiğinde biter ([GIT.tr.md](GIT.tr.md)). Git kavramları çekirdeğe değil bir eklentiye aittir (VISION.tr.md 1. ilke), ama eklenti sistemi aynı aşamada daha sonra gelir. Git ref adları, Apeiron ref adlarının izin vermediği karakterlere izin verir (ADR-013).

**Karar.** Hizmetlerin yanında ve çekirdeğin dışında yeni bir crate, `apeiron-git`, `gix` ile içe ve dışa aktarır. Commit'ler ve açıklamalı etiketler snapshot olur; snapshot alanlarının tutamadığı her şey, örneğin commit'i yazan, saat dilimleri ve imzalar, snapshot'ın `meta`'sında ham başlık satırları olarak saklanır. Dışa aktarma her nesneyi ham baytlardan yeniden kurar ve eski kimliğini aldığını denetler. Git ref adları kaçışlanarak Apeiron ref adlarına çevrilir. Crate, eklenti arayüzü olunca onun arkasına taşınır.

**Gerekçe.** Ham başlıkları tutmak, çekirdeğe Git hakkında hiçbir şey öğretmeden, Git'in kendisinin yazdığı her commit için, imzalılar da dahil, gidiş-dönüşü kesin yapar. Ayrı bir crate, `gix`'i diğer her crate'in dışında tutar ve ileride eklenti olarak sarmalanması kolaydır. Eklenti sistemini beklemek, aşamanın çıkış kriterini en büyük işinin arkasında bekletirdi.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Yalnızca yazar, zaman ve mesajı eşlemek | Commit'i yazan, saat dilimleri ve imzalar kaybolur; dışa aktarılan her commit yeni bir kimlik alır |
| Ham Git nesnelerini snapshot'ların yanında saklamak | Tüm üst verinin ikinci bir kopyası; snapshot'lar tek başlarına kullanılamazdı |
| Ref adlarında büyük harflere ve daha çok karaktere izin vermek | Ref'ler dosyadır; yalnızca harf büyüklüğüyle ayrılan adlar Windows ve macOS'ta çakışır |
| İçe almayı önce bir WebAssembly eklentisi olarak yazmak | Önce eklenti sistemi ve bir dosya sistemi arayüzü gerekir; çıkış kriterini bekletir |

**Sonuçlar.** Git'ten alınan snapshot'lar `meta` içinde `git.*` anahtarları taşır. Tree girdileri zaman tutmaz; bu yüzden bir Git snapshot'ını geri yüklemek dosyalara geri yükleme anının zamanını verir. Dışa aktarma gevşek nesneler yazar. SHA-256 depoları ve sığ klonlar şimdilik reddedilir.

## ADR-022: Dallar ref'tir; senkron ve dallar için tek bir üç yönlü birleştirme

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Aşama 3 dallanma ve birleştirme ekler ([BRANCH.tr.md](BRANCH.tr.md)). Ref'ler zaten her türden snapshot'ı adlandırır (ADR-013) ve senkron zaten iki geçmiş çizgisini yol başına, çatışma kopyalarıyla birleştirir (ADR-020).

**Karar.** Bir dal sıradan bir ref'tir; `ref create` ve `ref delete` onları yönetir. `merge` bir snapshot'ı bir ref'e birleştirir: yapılacak bir şey yoktur, bir ileri sarmadır ya da ikisini de ebeveyn alan yeni bir snapshot'tır; bu snapshot, artık hizmetlerin kendi modülünde duran, senkronun kullandığı aynı yol başına üç yönlü birleştirmeyle yapılır. Bir dosyanın iki farklı sürümü de tutulur; gelen `name (conflict <from>).ext` olarak. Bir birleştirme yalnızca snapshot'lar üzerinde çalışır; klasörleri `restore`, `backup` ve senkron değiştirir.

**Gerekçe.** Tek bir birleştirme, cihazlar ve dallar için aynı, test edilecek ve açıklanacak tek bir kural kümesi demektir. Çatışma kopyaları bir birleştirmeyi asla durdurmaz ve hiçbir şey kaybettirmez; onları çözmek özel bir araç istemez. Klasörleri birleştirmenin dışında tutmak, güvenlik kuralları bunun için yapılmış senkronun yanında, kullanıcının dosyalarını değiştirmenin ikinci bir yolunu önler.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Ref'lerin yanında ayrı bir dal türü | Aynı şey için iki kavram; ADR-013 ref'leri her amaca hizmet eder kıldı |
| Git'teki gibi bir birleştirmeyi çatışmada, çözülene kadar durdurmak | Yarım kalmış bir birleştirme için bir durum ve onu bitirecek bir araç gerekir; çatışma kopyaları ikisini de gerektirmez |
| Metni şimdi satır satır birleştirmek | Tür sürücülerine aittir (VISION.tr.md); onu metin diff eklentisi getirir |
| Bir dalı izleyen çalışma klasörü (`checkout`) | Kullanıcı dosyalarını değiştirmenin ikinci bir yolu; şimdilik ihtiyacı `restore`, `backup` ve senkron karşılar |

**Sonuçlar.** Bir metin dosyasının farklı satırlarındaki iki düzenleme, metin diff eklentisi olana kadar bir çatışmadır. Çatışma kopyaları birleştirilmiş snapshot'ta belirir ve sonraki değişiklikle temizlenir. Rebase ve cherry-pick yoktur.

## ADR-023: Arama saklanan bir dizin olmadan snapshot'ları okur

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Aşama 3, tek bir snapshot'ta ya da tüm geçmişte ada, üst veriye ve metne göre arama ekler ([SEARCH.tr.md](SEARCH.tr.md)). VISION.tr.md çevrimdışı çalışan ve gerektiğinde yeniden kurulan bir dizin ister. Depolar şifreli olabilir ve ref'ler yedeklerle, pull'larla, birleştirmelerle ve çöp toplamayla yer değiştirir.

**Karar.** Bir arama snapshot'ların kendisini en yeniden eskiye okur. Daha yeni bir snapshot'ta aynı yolda görülmüş bir klasör yeniden dolaşılmaz ve bir dosyanın metni içerik başına bir kez okunur. Bir yolun her sürümü, onu tutan en yeni snapshot ile bir kez raporlanır. Metin, 16 MiB'a kadar olan UTF-8 dosyalarda aranır.

**Gerekçe.** İçerik adresleme, bir dizinin vereceğini zaten verir: değişmemiş klasörler ve içerikler aynı kimliği paylaşır; böylece 3568 Git commit'lik bir geçmiş ada göre saniyenin bir kesrinde, metne göre birkaç saniyede aranır. Dizin olmayınca şifrelenecek ikinci bir ad ve metin kopyası, ref'lerle güncel tutulacak bir şey de yoktur.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Tam metin dizin dosyası gibi saklanan bir dizin | Şifrelenecek, her değişiklikten sonra güncellenecek ve onarılacak ikinci bir depo; bugünkü hızda gerekmiyor |
| Yalnızca en yeni snapshot'ta aramak | Bir dosyanın geçmişte nerede olduğunu bulmak belirtilmiş bir hedeftir (VISION.tr.md) |
| Her biçimi şimdiden metin için okumak | Eklenti sistemiyle gelen tür sürücülerini gerektirir |

**Sonuçlar.** Metin arama, karşılaştığı her farklı metin dosyası içeriğini okur; bu yüzden geçmişteki metin miktarıyla büyür. Ölçümler gerektirirse aynı komutun arkasına saklanan bir dizin sonradan eklenebilir.

## ADR-024: Python SDK'sı C ABI'yi ctypes ile çağırır

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Aşama 3, C ABI'yi ve ilk SDK'yı, Python için, ekler ([SDK.tr.md](SDK.tr.md)). Her SDK diğerleriyle aynı sonucu vermelidir (ADR-015) ve proje sahibi birkaç dildeki kodu elle sürdürmez.

**Karar.** Python SDK'sı, `libapeiron`'u `ctypes` ile yükleyen ve C ABI işlevlerini çağıran tek bir saf Python modülüdür; yalnızca dönüşümleri, hata türü başına bir istisna sınıfını, context manager'ları ve bir ilerleme yineleyicisini ekler. `sdk/conformance` içindeki aynı JSON senaryoları, CI'da her işletim sisteminde Rust'tan C işlevleri üzerinden ve Python'dan SDK üzerinden çalışır. C başlık dosyasını `cbindgen` üretir; depodaki başlık farklıysa bir test başarısız olur.

**Gerekçe.** `ctypes` Python'un parçasıdır; bu yüzden SDK derleyici gerektirmez ve aynı dosya her Python sürümü ve işletim sisteminde çalışır; tek ikili dosya yerel kütüphanedir. C ABI'nin etrafından değil içinden geçmek, diğer her dilin kullanacağı ABI'yi sınar. Ortak senaryolar "her dilde aynı sonuç"u bir sözden bir denetime çevirir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| PyO3 ile derlenen bir eklenti | C ABI'nin yanında kendine ait yapıştırma kodu ve her Python sürümü için ayrı derleme |
| `cffi` | `ctypes`'in bir düzine işlev için zaten yaptığı iş için ek bir bağımlılık |
| Elle yazılmış tipli sınıflar | Rust türlerinden ayrışırdı; JSON şemalarından üretilerek gelecekler |

**Sonuçlar.** Şemalar gelene kadar sonuçlar alan adıyla ya da anahtarla okunan kayıtlardır. Wheel'ler kütüphaneyi taşıyana kadar kütüphane `APEIRON_LIB` ile ya da modülün yanında bulunur.

## ADR-025: Eklentiler hiçbir şey verilmeden çalışır; önce tür sürücüleri gelir

**Durum:** Kabul edildi (2026-10-29)

**Bağlam.** Aşama 3, Wasmtime ve bileşen modeli üzerinde eklenti sistemini ekler ([PLUGIN.tr.md](PLUGIN.tr.md), ADR-005). İlk kullanıcısı, metni birleştiren ve gösteren metin diff eklentisidir. Eklentiler başkalarının yazdığı kodu çalıştırır ve bir birleştirme asla veri kaybetmemelidir. Rust 1.85 ile derlenen en yeni Wasmtime 2025 ortasında yayımlandı.

**Karar.** İlk eklenti arayüzü `apeiron:plugin/type-driver@0.1.0`'dır ve `describe`, `merge` ve `diff` içerir. Bir birleştirme, çatışma kopyası oluşturmadan önce deseni dosya adına uyan ilk kurulu eklentiye sorar; birleştirilmiş içerikten başka her şey çatışma kopyasını bırakır. Eklentiler kullanıcı başına kurulur. Her çağrı, WASI ile ama hiçbir şey verilmeden yeni bir örnekte çalışır: dosya, ağ, ortam ya da argüman yok, en çok 30 saniye ve 512 MiB; izin isteyen bir eklenti reddedilir. Güncel Wasmtime'ı kullanmak için en düşük Rust sürümü 1.85'ten 1.97'ye çıkar.

**Gerekçe.** Bir tür sürücüsü hemen işe yarar ve izin gerektirmez; böylece sandbox kapalı başlayabilir ve yalnızca ihtiyaç duyan eklenti türleri için, kullanıcının onayıyla açılır. Çatışma kopyasına geri dönmek, her başarısızlığı eklenti olmaması kadar güvenli tutar. Standart kütüphaneyle yazılmış eklentiler WASI'yi içe aktarır; bu yüzden boş bir WASI bağlamı onlara hiçbir şey vermeden çalışmalarını sağlar. Başkalarının kodu için bir sandbox güncel güvenlik düzeltmelerini taşımalıdır; bu, eski bir Rust ile derlenmekten ağır basar.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Depoda saklanan ve onunla kopyalanan eklentiler | Her cihaz bir uzak kopyadan aldığı kodu çalıştırırdı; kurmak her kullanıcının kararı olarak kalır |
| Şimdiden bir izin sorusu | Bu işin hiçbir eklenti türü izin gerektirmiyor |
| Süre sınırı yerine yakıt (fuel) | Yakıt zamanı değil komutları sayar; kullanıcı zamanı bekler |
| Rust 1.85 için en yenisi olan Wasmtime 34 | Sandbox'a bir yıldan fazla düzeltmeyi kaçırır |

**Sonuçlar.** Eklentisi olmayan bir cihaz, başka bir cihazın birleştirdiği yerde çatışma kopyası oluşturur; iki sonuç da güvenlidir. Apeiron'u derlemek Rust 1.97 ya da daha yenisini, eklenti ortamını sınamak `wasm32-wasip2` hedefini gerektirir. Fark işlevi, gösterilmek için metin diff eklentisini bekler.

## ADR-026: Yerelde sunulan ve yerel bir pencerede gösterilen tek bir web sayfaları takımı

**Durum:** Kabul edildi (2026-10-30)

**Bağlam.** Aşama 4, teknik bilgisi az olan kullanıcılar için bir masaüstü arayüzü ve ardından sunucu yöneticileri için bir web arayüzü gerektirir ([DESKTOP.tr.md](DESKTOP.tr.md), VISION.tr.md). İkisi de yalnızca herkese açık API'yi kullanmalıdır. Proje sahibi kodu elle sürdürmez; bu yüzden iki ayrı arayüz işi ikiye katlardı.

**Karar.** Arayüz, derleme adımı ve dış kütüphanesi olmayan, programın içine gömülen tek bir HTML, CSS ve JavaScript sayfaları takımıdır. `axum` üzerindeki yerel sunucu `apeiron-server` onları ve HTTP üzerindeki C ABI olan bir JSON API'yi yalnızca `127.0.0.1` üzerinde ve yalnızca adresin parça kısmında verilen rastgele bir jetona sahip olanlara sunar; `Host` ve `Origin` başlıklarını denetler. Masaüstü uygulaması sayfaları `wry` ve `tao` aracılığıyla işletim sisteminin web görünümünün bir penceresinde gösterir; `apeiron ui` onları bir tarayıcıda açar.

**Gerekçe.** 2. işin web arayüzü aynı sayfaları ve aynı JSON API'yi, oturum açmayla ağa sunarak yeniden kullanır. Web sayfaları ekran okuyuculara ve klavye kullanıcılarına erişilebilirdir ve ek iş olmadan açık ve koyu temada doğru görünür. Derleme adımı olmayınca güncel tutulacak bir JavaScript araç zinciri yoktur ve sayfalar herkesin okuyabileceği düz dosyalardır. `wry`, sistemin zaten sahip olduğu web görünümünü (WebKitGTK, WebKit, WebView2) kullanır; böylece tarayıcı dağıtılmaz; Tauri'nin temelidir ama Tauri'nin yapılandırması ve derleme araçları olmadan.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Yerel bir Rust araç takımı (egui, iced, Slint) | Web için yine ikinci bir arayüz gerekirdi; erişilebilirlik daha zayıf; Slint'in lisansı koşullar ekler |
| Tauri | Derleme araçları ve yapılandırması, programın kendi sunduğu sayfaların etrafındaki bir pencereye pek bir şey katmaz |
| Electron | Koca bir tarayıcı dağıtır; Raspberry Pi'de büyüktür |
| Derleme adımı olan bir JavaScript çatısı | Birkaç ekran için güncel tutulacak bir araç zinciri |

**Sonuçlar.** Masaüstü uygulamasını Linux'ta derlemek WebKitGTK geliştirme dosyalarını gerektirir. Sayfalar yalnızca program çalışırken çalışır. Masaüstü uygulamasının diske hiç yazmadığı jetonu okuyabilen herkes, kullanıcının açtığı depoyu kullanabilir.

## ADR-027: Web arayüzü tek bir depoyu TLS üzerinden parolayla oturum açana sunar

**Durum:** Kabul edildi (2026-10-30)

**Bağlam.** Aşama 4'ün 2. işi, masaüstü arayüzünün sayfalarını bir ev sunucusundaki depo için başka bilgisayarlara gösterir ([WEB.tr.md](WEB.tr.md)). Ağda yerel sunucunun jetonu yetmez: adrese ulaşan herkes deneyebilir, bir tarayıcı sayfalar boyunca oturumda kalmalıdır ve sunucunun bütün dosya sistemi oturum açan herkese açık olmamalıdır. Kendi hakları olan birden çok kullanıcı 3. iştir.

**Karar.** `apeiron web`, başlarken verilen ve CLI'nın diğer her komuttaki gibi açtığı depoyu sunar. Tarayıcı, yapılandırma klasöründeki `web.json` dosyasında Argon2id özeti olarak saklanan tek bir parolayla oturum açar; oturum açmak, `HttpOnly`, `Secure`, `SameSite=Strict` bir çerezde rastgele 256 bitlik bir oturum verir; oturum 12 saat boşta kalınca ya da 7 gün sonra biter. Art arda beş yanlış parola oturum açmayı bir dakika durdurur. TLS, verilen sertifikayla ya da bir kez oluşturulup parmak iziyle duyurulan kendinden imzalı bir sertifikayla `rustls` üzerinden gelir; `--plain` TLS'yi bir vekile bırakır. Yalnızca sayfaların kullandığı işlemlere izin verilir; yollara da bağlantılar çözüldükten sonra yalnızca `--folder` klasörlerinin içindeyse izin verilir.

**Gerekçe.** Başlarken sabitlenen depo, depo açmayı, oluşturmayı ve onların parolalarını ağdan uzak tutar ve şifreli bir deponun başında kimse olmadan `--password-file` ile açılmasını sağlar. Çerezli bir oturum, bir sırrı tutmak için betik gerektirmez ve başka siteler tarafından gönderilemez. Kendinden imzalı bir sertifika, ev sunucularında çoğu zaman olmayan bir alan adı olmadan TLS'yi çalıştırır; Caddy gibi bir vekil ise herkese açık bir sertifika ekleyebilir. İşlemlerin ve klasörlerin izin listesini denetlemek, bütün C ABI'nin her yolunu denetlemekten küçüktür.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Yerel sunucunun jetonu ağda | Adresteki uzun bir sır geçmişe ve yer imlerine kopyalanır; oturum kapatılamaz |
| HTTP temel kimlik doğrulaması | Tarayıcı parolayı her istekle gönderir ve oturum kapatamaz |
| Yalnızca düz HTTP, TLS her zaman bir vekilden | Vekil, bir ev kullanıcısının kurması gereken bir program daha demektir |
| Programın içinde Let's Encrypt sertifikaları | Herkese açık bir alan adı ve açık bir 80 kapısı ya da DNS erişimi ister; bugün bunu bir vekil yapar |
| C ABI'nin her işlemi | Uzak hedefler, dışa aktarma ve eklentiler izinli klasörlerin ötesindeki yollara ve programlara uzanır |
| Diskte bir oturum deposu | Sunucu yeniden başlayınca herkesin oturumunun kapanması kabul edilebilir ve çalınacak bir şey bırakmaz |

**Sonuçlar.** Durmadan tahmin eden biri, sahibinin oturum açmasını birer dakikalık sürelerle engelleyebilir. Oturum açan herkes, izinli klasörlerin içinde sunucuyu çalıştıran kullanıcının haklarına sahiptir. Tarayıcı kendinden imzalı bir sertifika için tarayıcı başına bir kez uyarır. Kullanıcılar ve roller (3. iş) tek parolanın yerini alacak.

## ADR-028: Üç rollü kullanıcılar, ref adıyla yedekler ve üst sürümler yoluyla görünürlük

**Durum:** Kabul edildi (2026-10-30)

**Bağlam.** Aşama 4'ün 3. işi, birkaç kişinin tek bir yedek sunucusunu paylaşmasını sağlar ([USERS.tr.md](USERS.tr.md)). Her biri yalnızca kendi yedeklerini ve klasörlerini görmeli ve geri yükleyebilmeli; biri de bütün sunucuyla ilgilenmelidir. Deponun tek bir şifreleme anahtarı vardır ve snapshot'lar ref adlarının yanında kimlikleriyle de bulunur; bu yüzden bir ref'i gizlemek yetmez.

**Karar.** `web.json` kullanıcılar dosyası, ADR-027'deki tek parolanın yerine adı, Argon2id parola özeti, üç sabit rolden biri (`admin`, `member`, `reader`), önek için `*` kullanılan bir ref adları listesi ve `--folder` klasörlerinin içinde klasörleri olan kullanıcılar koyar. Bir snapshot, kullanıcının ref'lerinden birinin en yeni sürümünden üst sürümler yoluyla ulaşılabiliyorsa o kullanıcıya görünürdür; bir isteğin adlandırdığı her snapshot çözülür, denetlenir ve kimliğiyle değiştirilir. Sunucu dosya değişince onu yeniden okur ve bir oturum başladığı andaki parola özetini tutar. İşler onları başlatan kullanıcıya aittir. Kullanıcılar `apeiron web user` ile yönetilir.

**Gerekçe.** Üç sabit rol, öğrenilecek bir kural dili olmadan bir aileyi ya da küçük bir ofisi karşılar. Ref adları yedeklerin zaten adlandırıldığı yoldur; böylece `backup/ayse-*` gibi bir önek, dosyayı düzenlemeden bir kişiye kendisinin her yeni yedeğini verir. Ulaşılabilirlik, bir kimliğin atlatamayacağı tek denetimdir ve kümeler ref başı başına tutulur; böylece bir sonraki yedeğe kadar bir geçmişin bir kez gezilmesine mal olur. Dosyayı yeniden okumak, CLI'nın sunucuyu yeniden başlatmadan kullanıcıları değiştirmesini sağlar; parola özetini oturumda tutmak da parola değişince çalınmış oturumları bitirir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Serbestçe birleştirilen işlem başına izinler | Üç rolden daha çok anlaşılacak ve yanlış yapılacak şey |
| İsteklerde yalnızca ref adlarını denetlemek | Snapshot kimlikleri ve `backup/x~1` gibi belirtimler geçerdi |
| Ayrı şifrelemeyle kullanıcı başına bir anahtar | Kullanıcılar arasında tekilleştirme durur, sunucu denetimler için yine her anahtara ihtiyaç duyar |
| Kullanıcılar için bir veritabanı | Bir ev sunucusu için küçük bir JSON dosyası yeter ve insanlar tarafından okunur |
| Kullanıcıları şimdi sayfalarda yönetmek | CLI bunu karşılar; denetim kaydı değişiklikleri kaydettiğinde sayfalar gelebilir |

**Sonuçlar.** Oturum açabilen herkes, deponun anahtarını sunucunun tutmasına güvenir; yani kullanıcılar birbirlerinden kriptografiyle değil sunucuyla korunur. Bir kullanıcının yedeğine birleştirilen bir snapshot o kullanıcıya görünür olur. Bir `member` ya da `reader` aramak için bir snapshot belirtmelidir. ADR-027'deki `--set-password` seçeneği kaldırıldı.

## ADR-029: Denetim kaydı, her şeyden önce yazılan, özet zincirli bir JSON satırları dosyasıdır

**Durum:** Kabul edildi (2026-10-30)

**Bağlam.** Aşama 4'ün 4. işi, web arayüzünde kimin ne yaptığını kaydeder ([AUDIT.tr.md](AUDIT.tr.md)). Bir admin kayda güvenebilmelidir: dosyaya erişim kazanan bir kullanıcı ya da bir hata onu fark edilmeden değiştirmemeli ve kayıtta olmayan hiçbir şey olmamalıdır. Hem sunucu hem CLI ona yazar.

**Karar.** Kayıt, durum klasöründeki tek bir JSON satırları dosyasıdır. Her satırda bir sıra numarası ve önceki satırın SHA-256 özeti vardır; `apeiron web audit verify` zinciri denetler ve son satırın özetini yazar. Bir satır, bir dosya kilidi altında eklenir ve istek sürmeden önce diske aktarılır; kısa bir işlem çalıştıktan sonra ve sonucu gönderilmeden önce, uzun bir işlem denetimlerinden sonra ve başlamadan önce kaydedilir. Kaydetmek başarısız olursa istek de başarısız olur. Adminler kaydı bir Etkinlik sayfasında ve `apeiron web audit show` ile okur.

**Gerekçe.** Bir özet zinciri, bir dosya ve bir özet işlevinden fazlası olmadan değiştirilmiş, silinmiş ve yeri değiştirilmiş satırları gösterir; başka bir yerde tutulan son özet ise yeniden yazılmış ya da kısaltılmış bir kaydı da gösterir. JSON satırları her araçla okunabilir ve yeniden yazmadan eklenebilir. Kaydı eksiksiz kılan, kapalı kalarak başarısız olmaktır: kaydedilemeyen bir işlem olmaz. Standart kütüphanenin kilidi, sunucu çalışırken CLI'nın kullanıcı değişikliklerini kaydetmesini sağlar.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Kaydı deponun içinde snapshot olarak tutmak | Her sayfa görüntüleme bir snapshot yazardı; deponun geçmişi kayıtlarla dolardı |
| syslog ya da journald | Her sistemde yok, değişiklik belli olmaz ve sayfalar daha çok hak olmadan okuyamaz |
| Her satırı bir anahtarla imzalamak | Anahtarı sunucu tutardı; dosyayı değiştirebilen genellikle imzalayabilir de; zincir ve başka yerde tutulan bir özet aynı durumları karşılar |
| Okumaları değil yalnızca değişiklikleri kaydetmek | "Dosyalarıma kim baktı?" kaydın yanıtlaması gereken sorulardan biridir |
| Uzun bir işlemi bittikten sonra kaydetmek | Bir çökme ya da kaybolan bir sonuç bir geri yüklemeyi kayıtsız bırakırdı |

**Sonuçlar.** Web arayüzündeki her istek diske bir yazmayı bekler. Döndürme eklenene kadar kayıt sınırsız büyür. Dosyaya erişimi olan biri kaydı baştan sona yeniden yazabilir; bunu yalnızca başka yerde tutulan bir özet ortaya çıkarır.

## ADR-030: Paylaşım, yalnızca özetleri tutulan, sabit bir sürüme süreli bağlantılarladır

**Durum:** Kabul edildi (2026-10-30)

**Bağlam.** Aşama 4'ün 5. işi, web arayüzünün bir kullanıcısının bir yedeğin sürümünü, klasörünü ya da dosyasını sunucuda hesabı olmayan birine vermesini sağlar ([SHARE.tr.md](SHARE.tr.md)). Alan kişinin bir tarayıcıdan başka bir şeye ihtiyacı olmamalı; paylaşan kişi paylaşımı bitirebilmeli; sunucunun dosyalarının sızan bir kopyası bağlantıları açmamalı ve paylaşım kullanıcının görebildiğinin ötesine geçmemelidir.

**Karar.** Bir bağlantı 32 rastgele bayttan oluşan bir gizli kısım taşır; sunucu yalnızca onun SHA-256 özetini; sahibi, sürümün kimliği, yol ve 1 ile 30 gün sonraki bir bitişle birlikte, durum klasöründeki, bir kilit altında değiştirilen ve bütünüyle yenisiyle değiştirilen `shares.json` dosyasında tutar. Her kullanım, bağlantının bitmediğini ve sahibinin hâlâ var olduğunu, `reader` olmadığını ve sürümü hâlâ gördüğünü denetler. Bağlantıyı açan kişi yalnızca paylaşılan yolun altında listeleyebilir ve indirebilir; dosyalar yeni `Repository::read_file` ile akış olarak gönderilir. Her kullanım, gizli kısım olmadan denetim kaydına yazılır.

**Gerekçe.** Rastgele bir gizli kısım ne bir hesap ne de yazılacak bir parola gerektirir ve 256 bit tahmin edilemez. Yalnızca özetini tutmak, parolalarda olduğu gibi, dosyanın bir kopyasının hiçbir şey açmaması demektir. Sabit bir sürüm, bağlantının tam olarak oluşturulurken denetleneni göstermesini sağlar. Sahibini her kullanımda denetlemek, bağlantının sahibinin haklarından uzun yaşamak yerine onları izlemesini sağlar. Kendine ait küçük bir dosya bağlantıları kullanıcılardan ayrı tutar; birini kaldırmak hiçbir zaman diğerine dokunmaz.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Durum tutmayan imzalı bağlantılar (yol ve bitişin HMAC'i) | Tek bir bağlantı, yine de bir liste tutmadan, bitmeden kaldırılamazdı |
| Kullanıcılar dosyasında konuk hesapları | Alan kişinin bir ada ve parolaya ihtiyacı olurdu ve admin sonradan temizlik yapmak zorunda kalırdı |
| Bir arşivi dışa aktarıp dosyayı sunmak | Her paylaşım için veriyi sunucuda kopyalar ve indirmeden önce klasörü göstermez |
| Bir yedeğin en yeni sürümünü izleyen bağlantılar | Paylaşılan şey denetlendikten sonra değişirdi; kullanıcı hiç bakmadığı dosyaları paylaşırdı |
| Bağlantının yeniden gösterilebilmesi için gizli kısmı tutmak | Dosyayı okuyabilen herkes her bağlantıyı kullanabilirdi |

**Sonuçlar.** Kaybolan bir bağlantı yeniden gösterilemez; yenisi oluşturulmalıdır. Bağlantıya sahip olan herkes, bitene kadar onun gösterdiğini okuyabilir; bu yüzden paylaşanın güvendiği bir yoldan gönderilmelidir. Her kullanım paylaşımlar dosyasını ve kullanıcılar dosyasını okur; bu bir ev ya da küçük ofisin sayıları için sorun değildir. Arşivler eklenene kadar klasörler dosya dosya indirilir.

## ADR-031: Merkezi kimlik, kullanıcıları hâlâ web.json içinde listelenen OpenID Connect'tir

**Durum:** Kabul edildi (2026-10-30)

**Bağlam.** Aşama 5'in 1. işi, bir kurumun çalışanlarının web arayüzünde zaten sahip oldukları hesapla oturum açmasını sağlar ([IDENTITY.tr.md](IDENTITY.tr.md)). Kurumlar pek çok kimlik ürünü kullanır; sunucu yaygın olanlarla çalışmalı, sahte ya da yeniden oynatılan bir oturum açmayı geçirmemeli ve sağlayıcı çalışmadığında da ulaşılabilir kalmalıdır.

**Karar.** Sunucu, PKCE'li yetkilendirme kodu akışını, kısa ömürlü bir çerezle tarayıcıya bağlanan bir state'i ve bir nonce'u kullanan bir OpenID Connect güvenen tarafıdır (relying party). Sağlayıcının ayarlarını ve anahtarlarını yayıncının bilinen adresinden okur ve ID belirtecini kendisi denetler: yayımlanmış bir anahtarla RS256 ya da ES256 imzası, yayıncı, hedef, süre ve nonce. Ayarlanmış tek bir alan, varsayılan olarak `email`, kullanıcıyı adlandırır; kullanıcı o kimlikle `web.json` içinde listelenmiş olmalıdır; rol, yedekler ve klasörler orada kalır. Bir kullanıcı parolasını tutabilir; böylece sağlayıcıya ulaşılamadığında bir `admin` oturum açabilir.

**Gerekçe.** OpenID Connect'i, LDAP ya da Active Directory'yi arkalarına koyanlar da dahil her yaygın sağlayıcı konuşur; bu yüzden tek bir protokol hepsine ulaşır. PKCE'li kod akışı belirteçleri tarayıcının adres çubuğundan uzak tutar; belirteci sunucuda sağlayıcının anahtarlarıyla denetlemek, ağ yoluna TLS'in ötesinde güvenmemek demektir. Yalnızca iki asimetrik algoritmayı kabul etmek, JSON Web Token'ların bilinen `none` ve paylaşılan gizli anahtar karışıklıklarını dışarıda bırakır. Kullanıcıları `web.json` içinde listelemek, sağlayıcıdaki bir hesabın tek başına yedeklere erişim vermemesi demektir; kimin neyi göreceğine yine admin karar verir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Bir dizine LDAP bind | Parolalar yedek sunucusundan geçerdi, ikinci adımlar kaybolurdu ve bulut sağlayıcıları bunu sunmaz |
| SAML | XML imzalarını güvenle denetlemek zordur ve SAML konuşan her sağlayıcı OpenID Connect de konuşur |
| Genel bir OpenID Connect kütüphanesi | İkinci bir HTTP yığını ve burada kullanılmayan pek çok özellik getirir; gereken denetimler, zaten kullanılan şifreleme kütüphanesiyle küçük bir modüle sığar |
| Sağlayıcının onayladığı her hesaptan kullanıcı oluşturmak | Kurumdaki herkes sunucuya ulaşabilirdi; rollere eşlenen gruplar sonra gelebilir |
| Oturum açmayı önde bir vekil sunucuya yaptırmak | Çalıştırılacak ve güvenilecek başka bir program gerekir ve denetim kaydı kimin oturum açtığını artık görmezdi |

**Sonuçlar.** Admin sunucuyu sağlayıcıya kaydeder ve gizli anahtarı, yalnızca sahibinin okuyabildiği `web.json` içinde tutar. Sağlayıcıda kapatılan hesapların zaten açık oturumları bitene kadar sürer. Yalnızca RS256 ve ES256 belirteçleri çalışır; başka bir şeye ayarlanmış bir sağlayıcı değiştirilmelidir. Kullanıcılar hâlâ `apeiron web user add --oidc` ile tek tek eklenir.

## ADR-032: Kiracılar ana makineyle ayrılır, her birinin kendi deposu ve dosyaları vardır

**Durum:** Kabul edildi (2026-10-30)

**Bağlam.** Aşama 5'in 2. işi, tek bir web sunucusunun birbirini görmemesi gereken birkaç kuruma ya da ekibe hizmet etmesini sağlar ([TENANTS.tr.md](TENANTS.tr.md)). Roller ve yedek kalıpları bir sunucunun kullanıcılarını zaten ayrı tutar (USERS.tr.md); ancak o zaman tek bir yanlış kalıp, bir denetimdeki tek bir hata ya da paylaşılan tek bir şifreleme anahtarı bir kurumu diğerine açardı. Her kurum için ayrı bir sunucu programı çalıştırmak işe yarar; ama her birinin bakılacak kendi portu, sertifikası ve hizmeti olur.

**Karar.** Tek bir süreç tek bir portu dinler ve `tenants.json` içinde listelenen her kiracıyı sunar. Her isteğin `Host` başlığı kiracıyı seçer; bilinen hiçbir ana makineye ait olmayan bir istek, herhangi bir kiracı onu görmeden `421` ile reddedilir. Her kiracı, tek bir web sunucusuyla aynı kodla kurulan, kendine ait eksiksiz bir sitedir: kendi deposu ve anahtarı, kullanıcı dosyası, kimlik sağlayıcısı, klasörleri, denetim kaydı, paylaşım bağlantıları, oturumları ve işleri. Kiracılar arasında program, port ve sertifika dışında hiçbir şey paylaşılmaz.

**Gerekçe.** Kiracıları ana makineyle ayırmak, her birine kendi tarayıcı kökenini (origin) verir; böylece çerezler, sayfalar ve kimlik sağlayıcısının geri dönüş adresi yalnızca sunucunun denetimleriyle değil, tarayıcının kendisi tarafından da ayrılır. Her kiracıya kendi deposunu vermek sınırı depolamaya da koyar: bir roldeki hata, başka bir anahtarla başka bir depoda duran veriye ulaşamaz. Her kiracıyı tek bir sunucunun sitesiyle aynı biçimde kurmak ikisi için tek bir kod yolu tutar; böylece USERS.tr.md, AUDIT.tr.md, SHARE.tr.md ve IDENTITY.tr.md'deki her denetim bir kiracı için değişmeden geçerlidir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Tek bir `web.json` ve tek bir depoda her kullanıcıya bir kiracı alanı | Her denetimin kiracıyı hatırlaması gerekirdi, unutulan tek bir denetim veri sızdırırdı ve tüm kiracılar tek bir şifreleme anahtarını paylaşırdı |
| Kiracıları `/t/acme/` gibi bir yolla ayırmak | Tüm kiracılar tek bir kökeni paylaşırdı; bu yüzden birinin sayfası diğerinin çerezlerini okuyabilir ve API'sini çağırabilirdi |
| Bir vekil sunucunun arkasında kiracı başına bir sunucu programı | Bu iş olmadan da çalışır, ama kiracı başına bir port, bir hizmet ve bir yeniden başlatma gerektirir; tercih eden için hâlâ mümkündür |
| Ref'leri kiracı başına önekli, paylaşılan tek bir depo | Kiracılar arası tekilleştirme, başka bir kiracının aynı dosyayı tutup tutmadığını gösterirdi ve tek bir anahtar hepsini açardı |

**Sonuçlar.** Her kiracının sunucuya ulaşan bir ana makine adına ve onun için geçerli bir sertifikaya ihtiyacı vardır; bir joker (wildcard) sertifika hepsini kapsar. Kendinden imzalı sertifika, yeni bir ana makineli bir kiracı eklenince yeniden oluşturulur; bu yüzden parmak izi değişir. Kiracılar depolamayı paylaşamaz; bu yüzden iki kiracının tuttuğu aynı dosya iki kez saklanır. Sunucunun tamamı tek bir süreçtir: bir yeniden başlatma her kiracıyı bir an durdurur ve kiracı başına sınırlar henüz mümkün değildir.

## ADR-033: Replikasyon her ref'i uzak bir kopyaya push etmektir; görevi devralmak admin'in kararıdır

**Durum:** Kabul edildi (2026-10-30)

**Bağlam.** Aşama 5'in 3. işi her yedeği alındıktan kısa süre sonra ikinci bir yerde tutar ve ilk sunucu çöktüğünde ikinci bir sunucunun görevi devralmasını sağlar ([REPLICATION.tr.md](REPLICATION.tr.md)). Uzak kopyalar, push ve pull zaten vardır (FORMAT.tr.md §15) ve bir uzak kopya eksiksiz bir depodur; ama onlar bir seferde bir ref taşır ve hiçbir şey bir kopyayı güncel tutmuyor ya da sunmuyordu.

**Karar.** Replika sıradan bir uzak kopyadır ve her ref'i ona push ederek güncel tutulur: `push --all` ile, zamanlanmış bir işle ya da web sunucusunun her yedekten hemen sonra ve ref'ler değiştiğinde her 30 saniyede bir yaptığı push'la. Replikanın ilerlediği bir ref bildirilir, diğerleri yine gider. Yedek sunucu, bir replika üzerinde `--standby` ile başlatılan web sunucusudur: yedek almak dışında her şey çalışır ve yeni yedekleri push edildikçe gösterir. `GET /api/health` bir izleyiciye deponun okunup okunamadığını ve sunucunun rolünü söyler. Görevi devralmayı admin, yedek sunucuyu `--standby` olmadan yeniden başlatarak yapar.

**Gerekçe.** Uzak kopyaları kullanmak tek bir format, tek bir bütünlük kuralları kümesi ve test edilmiş tek bir kod yolu demektir: push ve pull'un hata enjeksiyonu testleri replika için de geçerlidir, bir replikadan başka hiçbir şey olmadan geri yükleme yapılabilir ve şifreli bir depo onda da şifreli kalır. Her yedekten sonra push etmek replikayı ikinci bir süreç olmadan bir push geride tutar; dönemsel tur geri kalan her şeyi kapsar. Yedek sunucuda yedek almayı reddetmek ve görev devri kararını bir kişiye bırakmak, iki sunucunun asla aynı anda aynı geçmişe yazmaması demektir; ref'lerdeki karşılaştır-ve-değiştir (compare-and-swap) de eski bir birincilin geri dönüşünü kaybolan yedeklere değil, bildirilen bir çatışmaya çevirir.

**Değerlendirilen alternatifler.**

| Alternatif | Neden seçilmedi |
| --- | --- |
| Deponun dosyalarını rsync ya da replikalı bir dosya sistemiyle kopyalamak | Dosyaları herhangi bir sırayla kopyalar; böylece kopyalamanın ortasında yakalanan bir replikada bir ref, nesnelerinden önce bulunabilir; bozulmayı da kopyalar |
| Birkaç sunucu arasında bir uzlaşma protokolü (Raft) | Bir pilotun ihtiyaç duymadığı bir şey için çok daha fazla kod ve en az üç sunucu; her yazma bir çoğunluğu beklerdi |
| Başarısız bir sağlık denetiminde otomatik görev devri | Bir ağ bölünmesi, ikisi de yedek alan iki birincil oluştururdu; bir admin'in bir dakikasının bedeli daha küçüktür |
| Ref'leri replikada da silmek | Birincildeki bir hata ya da saldırı replikayı da boşaltırdı |

**Sonuçlar.** Replika bir yedekten sonra bir push kadar ya da bir işin aralığı kadar geridedir. Kullanıcılar, kimlik sağlayıcısı ve kiracılar admin tarafından yedek sunucuya kopyalanır; denetim kaydı ve paylaşım bağlantıları her sunucuda kalır. Görev devri bir kişi gerektirir. Birincilde silinen ref'ler, orada kaldırılana kadar replikada kalır.
