> **English:** [GLOSSARY.md](GLOSSARY.md)

# Apeiron Sözlüğü

Apeiron belgelerinde ve kodunda geçen terimler ve İngilizce karşılıkları. Türkçe belgelerde İngilizce sözcüğün aynen kullanıldığı yerlerde (ör. *chunk* ya da *snapshot*) Türkçe sütunu bunu gösterir.

## 1. Nesne modeli

| İngilizce | Türkçe | Tanım |
| --- | --- | --- |
| Blob | blob | Bir dosyanın içeriğini oluşturan sıralı chunk listesi |
| Chunk | chunk (parça) | Dosyanın içerik tanımlı, yaklaşık 1 MiB'lık parçası; özetiyle adreslenir |
| Object | nesne | Saklanan her birim: chunk, blob, tree ya da snapshot. Yazıldıktan sonra değişmez |
| Object id | nesne kimliği | Bir nesnenin 32 baytlık kimliği: anahtarlı BLAKE3 özeti |
| Object type | nesne türü | Nesnenin chunk, blob, tree ya da snapshot olduğunu söyleyen tek bayt |
| Parent snapshot | ebeveyn snapshot | Bir snapshot'ın üzerine kurulduğu önceki snapshot; geçmişi oluşturur |
| Policy | politika (policy) | Veriden ayrı tutulan saklama, şifreleme, senkron ve doğrulama kuralları |
| Ref | ref | Bir snapshot'ı gösteren ad; dallar, etiketler ve yedek noktaları birer ref'tir |
| Reflog | reflog | Bir ref'in yalnızca sonuna eklenen değişiklik günlüğü; ref değişikliklerini geri almak için kullanılır |
| Repository | depo | `config` dosyası, paketler, indeksler, ref'ler ve reflog'ları tutan klasör |
| Snapshot | snapshot (anlık görüntü) | Bir veri kümesinin belirli bir andaki durumu; ebeveynleri, yazarı ve açıklamasıyla |
| Store | store (depolama hedefi) | Nesnelerin saklandığı yer: yerel disk, harici disk, ağ ya da uzak hedef |
| Tree | tree (dizin ağacı) | Tek bir dizin: adları, türleri, izinleri, zamanları ve öznitelikleriyle girdiler |
| Type driver | type driver (tür sürücüsü) | Türe özgü diff, birleştirme, önizleme ve indeksleme ekleyen eklenti |

## 2. Depolama ve bütünlük

| İngilizce | Türkçe | Tanım |
| --- | --- | --- |
| Append-only | yalnızca eklemeli | Yeni veri kabul eden ama var olanı hiç değiştirmeyen ya da silmeyen saklama |
| Atomic rename | atomik yeniden adlandırma | Bir dosyayı tek adımda değiştirmek; okuyan ya eski ya yeni dosyayı görür |
| BLAKE3 | BLAKE3 | Apeiron'daki tüm özetleme işlerinde kullanılan kriptografik özet algoritması |
| Canonical encoding | kanonik kodlama | Aynı değerin her zaman aynı baytları verdiği kodlama |
| CBOR | CBOR | Concise Binary Object Representation (RFC 8949); blob, tree ve snapshot için kullanılır |
| Compare-and-swap (CAS) | karşılaştır ve değiştir | Bir değeri yalnızca hâlâ beklenen eski değerdeyse güncellemek |
| Content addressing | içerik adresleme | Veriyi içeriğinin özetiyle adlandırmak |
| Content-defined chunking (CDC) | içerik tanımlı bölme | Chunk sınırlarını verinin kendisinden seçmek; böylece düzenlemeler az sınırı kaydırır |
| Corruption | bozulma | Özetine ya da yapısına artık uymayan saklanmış veri |
| Deduplication | tekilleştirme | Aynı veriyi yalnızca bir kez saklamak |
| FastCDC | FastCDC | Apeiron'un kullandığı içerik tanımlı bölme algoritması (2020 sürümü) |
| Fault injection | hata enjeksiyonu | Bir işlemi seçilen bir noktada kasıtlı olarak kesip bozarak yapılan test |
| `fsync` | `fsync` (diske sabitleme) | Yazılan veriyi önbelleklerden çıkarıp diske yazmaya zorlamak |
| Garbage collection | çöp toplama | Hiçbir ref'in ya da reflog'un artık ulaşmadığı nesneleri silmek |
| Hash | özet (hash) | Veriden hesaplanan sabit boyutlu parmak izi |
| Id key | kimlik anahtarı | Anahtarlı nesne kimlikleri için kullanılan, depo başına rastgele anahtar |
| Index file | indeks dosyası | Hangi nesnenin hangi pakette olduğunu kaydeden dosya; yeniden üretilebilen bir önbellek |
| Integrity | bütünlük | Verinin tam olarak saklandığı gibi olduğunun güvencesi |
| Keyed hash | anahtarlı özet | Gizli bir anahtara da bağlı olan özet |
| Lock file | kilit dosyası | İki yazıcının aynı anda bir ref'i değiştirmesini engellemek için tekil olarak oluşturulan dosya |
| Pack file | paket dosyası | Birçok nesneyi tutan, kendi BLAKE3 özetiyle adlandırılmış dosya |
| Scrub | tam doğrulama (scrub) | Saklanan her nesnenin zamanlanmış, tam doğrulaması |
| Torn write | yarım yazma | Çökme ya da elektrik kesintisiyle yarıda kalan, kısmi veri bırakan yazma |
| zstd | zstd | Paket dosyalarının içinde kullanılan sıkıştırma algoritması |

## 3. İşlemler

| İngilizce | Türkçe | Tanım |
| --- | --- | --- |
| Archive | arşiv | Bir veri kümesinin ya da sürümün tek, taşınabilir paketi |
| Backup | yedek | Kurtarma için verinin bağımsız bir kopyası |
| Branch | dal | Üstüne yeni snapshot'lar eklendikçe ilerleyen ref |
| Cherry-pick | seçerek taşıma (cherry-pick) | Seçilen tek bir değişikliği bir geçmiş çizgisinden diğerine uygulamak |
| Conflict | çatışma | Aynı veriye yapılan ve otomatik olarak birleştirilemeyen iki değişiklik |
| Export | dışa aktarma | Veriyi başka bir biçimde dışarı yazmak (düz dosyalar, tar/zip, Git) |
| Import | içe aktarma | Veriyi başka bir biçimden ya da sistemden içeri almak |
| Incremental backup | artımlı yedek | Yalnızca değişeni aktaran yedek |
| Merge | birleştirme | İki geçmiş çizgisini tek bir çizgide birleştirmek |
| Rebase | yeniden temellendirme (rebase) | Değişiklikleri farklı bir başlangıç noktasının üzerine yeniden uygulamak |
| Restore | geri yükleme | Bir snapshot'taki veriyi dosyalara geri yazmak |
| Restore test | geri yükleme testi | Bir yedeğin çalıştığını kanıtlamak için onu düzenli olarak ayrı bir alana geri yüklemek |
| Retention policy | saklama politikası | Kaç yedeğin ne kadar süre tutulacağına dair kurallar |
| Sync | senkron (senkronizasyon) | Veriyi birden fazla cihazda ya da hedefte aynı tutmak |
| Tag | etiket | Sabit bir snapshot'a ad veren ref |

## 4. Mimari ve araçlar

| İngilizce | Türkçe | Tanım |
| --- | --- | --- |
| ADR | ADR (mimari karar kaydı) | Architecture decision record: tek bir karar için kısa belge |
| C ABI | C ABI | Diğer dillerin çağırabildiği, C dilinde kararlı ikili arayüz |
| Cargo workspace | Cargo workspace (çalışma alanı) | Birlikte derlenip test edilen Rust crate'leri kümesi |
| CLI | CLI (komut satırı arayüzü) | Command-line interface |
| Client-side encryption | istemci tarafı şifreleme | Veriyi, kullanıcının cihazından çıkmadan önce orada şifrelemek |
| Component model | bileşen modeli | Tipli, dilden bağımsız bileşenler için WebAssembly standardı |
| Conventional Commits | Conventional Commits | Bir commit mesajı kuralı (`feat:`, `fix:`, `docs:` …) |
| Crate | crate | Bir Rust paketi: kütüphane ya da program |
| Daemon | arka plan hizmeti (daemon) | Arka planda çalışan program; ör. zamanlanmış yedekler için |
| FFI | FFI | Foreign function interface: diller arasında kod çağırma |
| gitoxide (`gix`) | gitoxide (`gix`) | Rust ile yazılmış bir Git uygulaması |
| Hook | hook (kanca) | Bir olay gerçekleştiğinde, ör. snapshot'tan sonra, çalışan eklenti |
| Importer / exporter | içe / dışa aktarıcı | Veriyi başka bir sistemden alan ya da oraya gönderen eklenti |
| MSRV | MSRV (en düşük Rust sürümü) | Minimum supported Rust version: desteklenen en düşük Rust sürümü |
| Plugin | eklenti | Çekirdeği değiştirmeden Apeiron'u genişleten isteğe bağlı kod |
| Sandbox | sandbox (yalıtım ortamı) | Bir eklentinin yalnızca izin verileni yapabildiği yalıtılmış ortam |
| SDK | SDK | Software development kit: Apeiron'u bir dilden kullanmak için kütüphane |
| Storage backend | storage backend (depolama arka ucu) | Yeni bir depolama hedefi ekleyen eklenti |
| Tokio | Tokio | Ağ, senkron ve arka plan hizmeti katmanlarında kullanılan asenkron çalışma zamanı |
| Wasmtime | Wasmtime | Eklentileri çalıştıran WebAssembly çalışma zamanı |
| WebAssembly (WASM) | WebAssembly (WASM) | Sandbox içinde çalışan taşınabilir ikili biçim |
| WTF-8 | WTF-8 | Windows dosya adlarını bayt olarak saklamak için kullanılan kodlama |
