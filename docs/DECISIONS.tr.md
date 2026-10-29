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
| [015](#adr-015-c-abi-sınırında-json-değerler) | C ABI | Küçük ve sabit bir işlev kümesi; istekler ve sonuçlar JSON | Önerildi |
| [016](#adr-016-senkron-herkese-açık-api-ve-iş-parçacığında-çalışan-işler) | Uzun işlemler | Senkron API; sorgulanan ilerlemeyle iş parçacığında çalışan işler | Önerildi |
| [017](#adr-017-şifreleme-için-xchacha20-poly1305-ve-argon2id) | Şifreleme | Veri için XChaCha20-Poly1305, parolalar için Argon2id | Kabul edildi |
| [018](#adr-018-uzak-depo-olarak-deponun-tam-kopyası-sftp-için-russh-s3-için-object_store) | Uzak hedefler | Uzak depolar deponun tam kopyasıdır; SFTP için `russh`, S3 için `object_store` | Kabul edildi |

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

**Durum:** Önerildi (2026-10-28)

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

**Durum:** Önerildi (2026-10-28)

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

**Sonuçlar.** Koşullu yazma desteği olmayan S3 uyumlu hizmetler reddedilir. SFTP, sunucunun `~/.ssh/known_hosts` içinde olmasını ister. SFTP ve S3 kopyalarının reflog'u yoktur. Bir uzak kopyanın çöp toplaması sonraki bir iştir; bir klasör kopyası, bir depo olarak açılarak bugün de toplanabilir.
