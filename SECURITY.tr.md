> **English:** [SECURITY.md](SECURITY.md)

# Güvenlik Politikası

## 1. Desteklenen sürümler

Apeiron erken geliştirme aşamasındadır ve henüz bir sürümü yayımlanmadı; şu anki aşamayı [docs/ROADMAP.tr.md](docs/ROADMAP.tr.md) gösterir. Güvenlik düzeltmeleri yalnızca `master` dalının en son hâline yapılır.

| Sürüm | Destekleniyor |
| --- | --- |
| `master` | Evet |
| Yayımlanmış sürümler | Henüz yok |

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
| Eklenti yalıtımı | Eklentiler, bildirilmiş ve kullanıcının onayladığı izinlerle WebAssembly sandbox'ında çalışır | Yerinde; eklenti API'sinin 0.2 sürümü hiçbir izin vermez ([PLUGIN.tr.md](docs/PLUGIN.tr.md)) |
| Güvenli varsayılanlar | Uzak erişim ve telemetri varsayılan olarak kapalıdır | Uygulanıyor (web sunucusu yalnızca istenince başlar, `apeiron ui` yalnızca bu bilgisayarda dinler ve telemetri yoktur) |
| Zorunlu üçüncü taraf yok | Veri hiçbir zaman zorunlu olarak üçüncü bir tarafa gönderilmez | Uygulanıyor |

Güvenlik modelinin tamamı [docs/ARCHITECTURE.tr.md](docs/ARCHITECTURE.tr.md) içindedir.
