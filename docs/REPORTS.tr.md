> **English:** [REPORTS.md](REPORTS.md)

# Uyumluluk Raporları

> **Durum: Tamamlandı.** Aşama 5'in 5. işinin tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §7): bir denetçinin yedekler hakkında sorduklarını yanıtlayan, birinin hatırladıklarından değil, depo ve web sunucusunun şu anki hâlinden oluşturulan tek bir belge. Arkasındaki karar ADR-035'tir ([DECISIONS.tr.md](DECISIONS.tr.md)).

ISO 27001, SOC 2 ve KVKK/GDPR gibi standartlar ve yasalar bir kurumdan verisinin yedeklendiğini, yedeklerin eksiksiz ve ayrı tutulduğunu, korunduğunu ve gerektiği kadar saklandığını ve kimin erişimi olduğunu göstermesini ister. Bunu her denetimde elle toplamak yavaştır ve kolayca yanlış yapılır.

## 1. Neyi yanıtlar

| Soru | Raporun neresinde |
| --- | --- |
| Ne yedekleniyor ve en son ne zaman? | Yedekler: her ref, en yeni snapshot'ı ve zamanı; `--stale-days`'ten uzun süredir yeni snapshot'ı olmayan ref'ler eskimiş olarak işaretlenir |
| Saklanan veri eksiksiz mi? | Depo: `--verify` ile saklanan her bayt okunur ve denetlenir |
| İkinci bir kopya var mı ve güncel mi? | Yedekler ve Uzak depolar: `--remotes` ile her ref her uzak depoyla karşılaştırılır |
| Korunuyor mu? | Depo: şifreleme, parolalar ve kurtarma anahtarları |
| Gerektiği kadar saklanıyor mu? | Yedekler ve Saklama kuralları ([ARCHIVE.tr.md](ARCHIVE.tr.md)) |
| Ona kim ulaşabiliyor ve ne yaptılar? | Web sunucusu, `--web` ile: kullanıcılar ve hakları, kimlik sağlayıcısı, denetim kaydının durumu ve sayıları ([AUDIT.tr.md](AUDIT.tr.md)) |

## 2. Rapor oluşturmak

```text
apeiron report --verify --remotes --web > rapor-2026-10.md
apeiron --json report --verify --remotes --web > rapor-2026-10.json
```

| Seçenek | Anlamı | Varsayılan |
| --- | --- | --- |
| `--verify` | `apeiron verify` gibi saklanan her baytı okur ve denetler; o kadar sürer | Kapalı |
| `--remotes` | Her ref'i her uzak depoyla karşılaştırır; uzak depolar yanıt vermelidir | Kapalı |
| `--stale-days <n>` | Bir ref'in eskimiş sayılması için yeni snapshot'sız geçen gün sayısı; 0 hiçbir şey denetlemez | 7 |
| `--web` | Web sunucusunun bölümünü ekler | Kapalı |
| `--tenant <ad>` | `--web` ile: dosyaları raporlanacak kiracı ([TENANTS.tr.md](TENANTS.tr.md)) | Yok |
| `--since <YYYY-AA-GG>` | `--web` ile: denetim olaylarını o günden itibaren sayar | 30 gün önce |

Rapor yalnızca okur; hiçbir şeyi değiştirmez. Metin biçimi, saklanmaya ya da PDF'e dönüştürülmeye hazır Markdown'dır; `--json` aynı içeriği başka programlar için verir. Zamanlanmış bir iş her ay bir tane oluşturabilir ([DAEMON.tr.md](DAEMON.tr.md) komutları yalnızca kendi işleriyle çalıştırır; bunun için sistemin zamanlayıcısını kullanın).

| Çıkış kodu | Anlamı |
| --- | --- |
| 0 | Sorun yok; uyarılar olabilir |
| 3 | En az bir sorun |
| 4 | `--verify` bozuk veri buldu |

## 3. İçerik

1. **Bulgular**, en ciddisi önce (§4).
2. **Depo**: format sürümü, şifreleme, kaç parola ve kurtarma anahtarı olduğu, `--verify` sonucu.
3. **Yedekler**: ref başına snapshot kimliği, en yeni snapshot'ın zamanı, geçmişindeki snapshot sayısı, bir saklama kuralının onu kaç gün tuttuğu ve her uzak depodaki durumu: `current` (güncel), `behind` (geride), `missing` (yok) ya da uzak depoda bu kopyada olmayan snapshot'lar varsa `other` (farklı).
4. **Saklama kuralları**, her birinin ne zaman eklendiğiyle.
5. **Uzak depolar** ve yanıt vermeyenler.
6. **Web sunucusu**, `--web` ile: rolü, nasıl oturum açtığı, yedekleri ve klasörleriyle her kullanıcı; kimlik sağlayıcısı; denetim kaydının özet zincirinin eksiksiz olup olmadığı, olay sayısı ve son olayın özeti; `--since`'ten itibaren oturum açma, başarısız oturum açma, başlatılan yedek ve geri yükleme, oluşturulan paylaşım bağlantısı ve başarısız replikasyon sayıları.

Kanıt snapshot kimlikleridir: bir kimlik, altındaki her baytı ve ondan önceki tüm geçmişi sabitler; bu yüzden sonraki bir rapor ya da bir replikada `apeiron verify`, aynı verinin hâlâ orada olduğunu gösterir. Son denetim olayının özeti de denetim kaydı için aynısını yapar: raporda tutulduğunda, sonra önceki hiçbir olayın değiştirilmediğini gösterir (AUDIT.tr.md §3).

## 4. Bulgular

| Düzey | Ne zaman |
| --- | --- |
| Sorun | `--verify` bozuk veri buldu; depo açılırken bozuk dosyalar bulundu; bir uzak depo yanıt vermedi; bir ref bir uzak depoda geride, yok ya da farklı; denetim kaydı eksiksiz değil |
| Uyarı | Bir ref eskimiş; depo şifreli değil; kurtarma anahtarı olmadan şifreli; hiç uzak deposu yok, yani her yedek tek bir yerde duruyor; denetim kaydının en eski parçaları kaldırılmış (AUDIT.tr.md §3) |

JSON'da her bulgunun bir `level` alanı, İngilizce bir `text` alanı ve bilgileriyle birlikte bir `code` alanı vardır; böylece bir program ya da sayfa bulguyu başka bir dilde söyleyebilir: `damaged` (`files`, `objects`), `damaged_on_open` (`files`), `remote_down` (`remote`, `message`), `copy` (`name`, `remote`, `state`), `stale` (`name`, `days`), `not_encrypted`, `no_recovery_key`, `no_remote`, `audit_trimmed` (`first`), `audit_broken` (`message`).

## 5. Sayfalarda, JSON API'de ve CLI'da

| Nerede | Ne |
| --- | --- |
| `apeiron report` | §2 |
| `report` (API.tr.md §8) | `verify`, `remotes` ve `stale_days` ile uzun işlem; web sunucusu bölümü dışında §3'teki her şeyle bir `ComplianceReport` döndürür |
| Rapor sayfası | Web arayüzünde bir `admin` için ve masaüstü arayüzünde: doğrulama ve uzak depolarla karşılaştırma seçeneğiyle §3'teki bulgular, depo ve yedekler; web sunucusu bölümü `apeiron report --web` ile alınır |

## 6. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Belirli bir standardın maddelerine eşlenmiş raporlar | Sonra, standardını adlandıran bir pilotla; §3'teki gerçekler böyle bir eşlemenin gösterdiği şeylerdir |
| Raporu imzalamak | Sonra; tuttuğu snapshot kimlikleri ve denetim özeti zaten denetlenebilir |
| Tüm kiracılar için tek seferde raporlar | Sonra; o zamana kadar `--tenant` ile kiracı başına bir rapor |

## 7. Crate'ler

| Crate | Görevi | Durum |
| --- | --- | --- |
| `apeiron` | `report`, `ComplianceReport` | Tamamlandı |
| `apeiron-cli` | `apeiron report`, web sunucusu bölümü, Markdown biçimi | Tamamlandı |
