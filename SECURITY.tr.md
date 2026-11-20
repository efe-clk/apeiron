> **English:** [SECURITY.md](SECURITY.md)

# Güvenlik Politikası

## 1. Desteklenen sürümler

Apeiron erken geliştirme aşamasındadır; ilk sürüm 0.1.0, 2026-11-18'de yayımlandı ve şu anki aşamayı [docs/ROADMAP.tr.md](docs/ROADMAP.tr.md) gösterir. Güvenlik düzeltmeleri yalnızca `master` dalının en son hâline yapılır; kullanıcılara bir sonraki sürümle ulaşır.

| Sürüm | Destekleniyor |
| --- | --- |
| `master` | Evet |
| Yayımlanmış sürümler | Hayır; düzeltmeler bir sonraki sürümle gelir |

## 2. Güvenlik açığı bildirme

Güvenlik sorunlarını lütfen herkese açık issue, pull request ya da tartışmalarda **bildirmeyin**.

1. Depoyu GitHub'da açın: <https://github.com/efe-clk/apeiron>.
2. **Security** sekmesine gidip **Report a vulnerability** seçeneğini kullanın (GitHub'ın gizli güvenlik açığı bildirimi).
3. Sorunu, etkilenen bileşeni ve mümkünse sorunu yeniden oluşturma adımlarını anlatın.

Bildiriminizi yalnızca proje yürütücüleri görür. En kısa sürede yanıt verilir; proje küçük bir ekip tarafından yürütüldüğü için sabit bir yanıt süresi sözü verilmez. Düzeltme hazır olunca sorun bir güvenlik duyurusuyla (security advisory) açıklanır; isterseniz adınız da belirtilir.

**Kapsamda** özellikle şu sorunlar vardır: sessiz veri kaybına ya da bozulmaya yol açabilecek, bütünlük doğrulamasını atlatan, saklanan içerik hakkında bilgi sızdıran ya da şifrelemeyi, izinleri veya eklenti sandbox'ını aşan sorunlar.

## 3. Güvenlik ilkeleri

| İlke | Kural | Durum |
| --- | --- | --- |
| Bütünlük | Her nesne kriptografik bir özetle (BLAKE3) doğrulanır; bozulma hiçbir zaman sessiz kalmaz | Tamamlandı |
| Kesinti güvenliği | Yaz → `fsync` → atomik yeniden adlandır; elektrik kesintisi hiçbir zaman yarım bir durumu görünür bırakmaz | Tamamlandı |
| İçerik gizliliği | Nesne kimlikleri anahtarlı özettir; saklanan içerik kimliklerden tahmin edilemez | Tamamlandı |
| Bellek güvenliği | Rust ile yazılır; `unsafe` yalnızca FFI crate'inde ve küçük `apeiron-sys` crate'inde | Uygulanıyor |
| Şifreleme | İstemci tarafında; anahtarlar kullanıcıya aittir; uzak hedef düz metni hiç görmez ([FORMAT.tr.md](docs/FORMAT.tr.md) §14) | Tamamlandı |
| İzinler | Okuma, yazma, silme, paylaşma ve yönetme izinleri ayrıdır | Web arayüzü için tamamlandı ([USERS.tr.md](docs/USERS.tr.md), [SHARE.tr.md](docs/SHARE.tr.md)) |
| Denetim kaydı | Hassas işlemler kaydedilir; hassas veri hiçbir zaman kaydedilmez | Web arayüzü için tamamlandı ([AUDIT.tr.md](docs/AUDIT.tr.md)) |
| Eklenti yalıtımı | Eklentiler, bildirilmiş ve kullanıcının onayladığı izinlerle WebAssembly sandbox'ında çalışır | Yerinde; tek izin, kancalar ve depolama arka uçları için kullanıcının kurulumda verdiği ağ ana makinelerine ulaşmaktır ([PLUGIN.tr.md](docs/PLUGIN.tr.md) §3.1) |
| Güvenli varsayılanlar | Uzak erişim ve telemetri varsayılan olarak kapalıdır | Uygulanıyor (web sunucusu yalnızca istenince başlar, `apeiron ui` yalnızca bu bilgisayarda dinler ve telemetri yoktur) |
| Zorunlu üçüncü taraf yok | Veri hiçbir zaman zorunlu olarak üçüncü bir tarafa gönderilmez | Uygulanıyor |

Güvenlik modelinin tamamı [docs/ARCHITECTURE.tr.md](docs/ARCHITECTURE.tr.md) içindedir.

## 4. Bir sürümü doğrulamak

0.6.0'dan itibaren bir sürümün her dosyası üç yolla denetlenebilir:

| Denetim | Nasıl | Neyi kanıtlar |
| --- | --- | --- |
| Sağlama toplamı | İndirilen dosyaların yanında `sha256sum -c SHA256SUMS --ignore-missing` | Dosya, sürüm sayfasının listelediğidir |
| Sigstore | `gh attestation verify <file> --repo efe-clk/apeiron` | Dosya, bu deponun `Release` iş akışı tarafından etiketinin commit'inden derlenmiş ve o zamandan beri değişmemiştir; imza Sigstore üzerinden kısa ömürlü bir sertifikayla atılır, bu yüzden çalınabilecek uzun ömürlü bir anahtar yoktur |
| Derleme kaydı | `BUILDINFO-linux-<mimari>.txt` | Yeniden derlemek için commit, Rust sürümü, derleme bayrakları ve Linux paketlerindeki programların sağlama toplamları |

macOS ve Windows paketleri bir Apple ya da Microsoft sertifikasıyla imzalı değildir, bu yüzden bu sistemler yine uyarır (§1); Sigstore kanıtı onları da kapsar.

Derlemeler tekrarlanabilir olacak biçimde yapılır: zaman commit'in zamanıdır (`SOURCE_DATE_EPOCH`), derleme makinesinin yolları `/build`, `/cargo` ve `/rustup` olarak eşlenir ve arşivler sıralı adlarla, kendi zamanları olmadan yazılır. `ape` iki derlemeden bayt bayt aynı çıkar. `apeiron` henüz çıkmaz: içerdiği WebAssembly çalışma ortamı Wasmtime 50, aynı klasörde bile bir derlemeden ötekine değişen kod üretir; bu yüzden aynı commit'in iki derlemesi orada farklıdır. Derleme kaydı, bu çözüldüğünde neyin karşılaştırılacağını listeler.

Depolama formatının çözücüleri (nesneler, dizin dosyaları, pack girdileri, ref, reflog ve işlem günlüğü satırları, `config` ve uzak depo adresleri) fuzz edilir: `fuzz/` her biri için bir hedef tutar ve CI her push'ta her birini bir dakika çalıştırır. Birini daha uzun çalıştırmak için: `cd fuzz && cargo +nightly fuzz run objects` (`cargo install cargo-fuzz` gerekir). Dış bir güvenlik denetimi yapılmadı; proje sahibinin seçeceği bir üçüncü taraf gerektirir.
