> **English:** [CLI.md](CLI.md)

# Apeiron Komut Satırı Arayüzü

> **Durum: İncelemeye sunulan taslak.** Bu belgedeki hiçbir şey henüz yazılmadı. `apeiron` komutunun Aşama 1 tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §3).

CLI, Apeiron'un ilk arayüzüdür. Diğer tüm arayüzler gibi yalnızca herkese açık API'yi kullanır ([API.tr.md](API.tr.md)); çekirdeğe üçüncü taraf bir programın sahip olmadığı hiçbir erişimi yoktur.

## 1. İlkeler

| İlke | Kural | Durum |
| --- | --- | --- |
| Yalnızca API | CLI yalnızca herkese açık `apeiron` crate'ini çağırır | Planlandı |
| Veri asla sessizce yok edilmez | Açık bir seçenek istemedikçe hiçbir komut var olan dosyaların üzerine yazmaz ya da onları silmez | Planlandı |
| Makinece okunabilir çıktı | Her komut `--json` kabul eder ve o zaman standart çıktıya tek bir JSON belgesi yazar | Planlandı |
| İlerleme | Uzun komutlar, standart hata bir terminalse ilerlemeyi orada gösterir | Planlandı |
| Güvenli iptal | Ctrl+C komutu durdurur; depo önceki durumunda kalır (eski ya da yeni, asla yarım) | Planlandı |
| Okunamayan dosyada devam | Okunamayan bir dosya raporlanır ve atlanır; yedek devam eder | Planlandı |
| Açık çıkış kodları | Her sonucun kendi çıkış kodu vardır (bölüm 5) | Planlandı |

## 2. Deponun seçilmesi

| Kaynak | Örnek | Öncelik |
| --- | --- | --- |
| `--repo` seçeneği | `apeiron --repo /mnt/backup/apeiron log` | 1 |
| `APEIRON_REPO` ortam değişkeni | `export APEIRON_REPO=/mnt/backup/apeiron` | 2 |

İkisi de verilmezse komut bir kullanım hatasıyla durur. Harici diskteki bir depo, o diskteki bir yolda duran sıradan bir depodur.

## 3. Komutlar

| Komut | Ne yapar | Durum |
| --- | --- | --- |
| `apeiron init <path>` | Boş bir depo oluşturur | Planlandı |
| `apeiron backup <folder> [--ref <name>] [--message <text>]` | Klasörü yeni bir snapshot olarak saklar ve ref'i ona taşır (varsayılan ref: `main`) | Planlandı |
| `apeiron log [<ref>]` | Bir ref'in snapshot'larını en yeniden başlayarak listeler | Planlandı |
| `apeiron ls <snapshot> [<path>]` | Bir snapshot'taki dosya ve klasörleri listeler | Planlandı |
| `apeiron diff <snapshot> <snapshot>` | İki snapshot arasında eklenen, silinen ve değişen dosyaları listeler | Planlandı |
| `apeiron restore <snapshot> <target> [--path <path>]` | Bir snapshot'ı (ya da içindeki bir yolu) yeni ya da boş bir klasöre yazar | Planlandı |
| `apeiron export <snapshot> <file>` | Bir snapshot'ı `.tar` ya da `.zip` dosyasına yazar | Planlandı |
| `apeiron refs` | Tüm ref'leri ve gösterdikleri snapshot'ları listeler | Planlandı |
| `apeiron verify` | Her paketi, indeksi ve nesneyi okuyup denetler | Planlandı |
| `apeiron repair` | Bayat ref kilitlerini kaldırır ve kaybolan reflog satırlarını ekler | Planlandı |
| `apeiron gc` | Hiçbir ref'in ya da reflog'un ulaşamadığı nesneleri siler | Planlandı (çöp toplama yazıldıktan sonra) |

Bir `<snapshot>`; ref adı (`main`), tam snapshot kimliği ya da tek bir snapshot'a uyuyorsa kimliğin ilk 8 ya da daha fazla karakteri olarak yazılabilir. `~N` eki ilk ebeveynler üzerinden N adım geri gider: `main~1`, `main`'den önceki snapshot'tır.

## 4. Bir yedeğin sakladıkları

| Öğe | Saklanır | Durum |
| --- | --- | --- |
| Dosyalar, klasörler, sembolik bağlar | Evet | Planlandı |
| İzin bitleri ve değişiklik zamanı | Evet | Planlandı |
| Linux ve macOS'ta sahip (uid/gid) | Evet | Planlandı |
| Genişletilmiş öznitelikler | Aşama 1'de değil | Planlandı |
| Sabit bağlar, aygıt dosyaları, soketler | Hayır; atlanır ve raporlanır ([FORMAT.tr.md](FORMAT.tr.md) §6.3) | Planlandı |

Okunurken değişen bir dosya okunduğu hâliyle saklanır ve uyarı olarak raporlanır. Geri yükleme izinleri ve zamanları yeniden oluşturur; sahipleri yalnızca bunları ayarlama yetkisiyle çalıştırıldığında.

## 5. Çıkış kodları

| Kod | Anlamı |
| --- | --- |
| 0 | Başarılı |
| 1 | Hata; depoda hiçbir şey değişmedi |
| 2 | Kullanım hatası (yanlış komut ya da seçenek) |
| 3 | Uyarılarla bitti (ör. atlanan dosyalar) |
| 4 | `verify` bozulma buldu |
| 130 | Ctrl+C ile iptal edildi |

## 6. Örnek oturum

```bash
apeiron init /mnt/backup/apeiron
export APEIRON_REPO=/mnt/backup/apeiron
apeiron backup ~/Documents --message "first backup"
apeiron backup ~/Documents
apeiron log
apeiron diff main~1 main
apeiron restore main /tmp/documents-restored
apeiron verify
```

## 7. Crate'ler ve kütüphaneler

| Crate | Görev | Durum |
| --- | --- | --- |
| `apeiron` | Herkese açık Rust API'si ([API.tr.md](API.tr.md) §8) | Planlandı |
| `apeiron-services` | API'nin kullandığı klasör ↔ snapshot, fark ve dışa aktarma mantığı | Planlandı |
| `apeiron-cli` | `apeiron` programı | Planlandı |

| İhtiyaç | Önerilen kütüphane | Gerekçe |
| --- | --- | --- |
| Komut satırı ayrıştırma | `clap` | En yaygın Rust ayrıştırıcısı; yardım metnini üretir ve seçenekleri denetler |
| JSON çıktı | `serde_json` | `serde` için standart JSON kütüphanesi; `serde` yapılandırmada zaten kullanılıyor |
| tar ve zip dışa aktarma | `tar`, `zip` | Yaygın kullanılan, saf Rust |

## 8. Açık sorular

| Soru | Öneri |
| --- | --- |
| `backup` için varsayılan ref | `main`; birden fazla klasör için `--ref backup/<name>` |
| `restore` boş olmayan bir klasöre yazabilir mi? | Aşama 1'de hayır; ileride bir `--overwrite` seçeneği buna izin verebilir |
| Aynı depoda eşzamanlı komutlar | Bir depo kilidi (Aşama 1, [FORMAT.tr.md](FORMAT.tr.md) §12) ikinci yazıcıyı bekletir ya da durdurur |
