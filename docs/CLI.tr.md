> **English:** [CLI.md](CLI.md)

# Apeiron Komut Satırı Arayüzü

> **Durum: Devam ediyor.** `apeiron` komutunun Aşama 1 tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §3). Tamamlandı olarak işaretlenen kısımlar bugün çalışır.

CLI, Apeiron'un ilk arayüzüdür. Diğer tüm arayüzler gibi yalnızca herkese açık API'yi kullanır ([API.tr.md](API.tr.md)); çekirdeğe üçüncü taraf bir programın sahip olmadığı hiçbir erişimi yoktur.

## 1. İlkeler

| İlke | Kural | Durum |
| --- | --- | --- |
| Yalnızca API | CLI yalnızca herkese açık `apeiron` crate'ini çağırır | Tamamlandı |
| Veri asla sessizce yok edilmez | Açık bir seçenek istemedikçe hiçbir komut var olan dosyaların üzerine yazmaz ya da onları silmez | Tamamlandı |
| Makinece okunabilir çıktı | Her komut `--json` kabul eder ve o zaman standart çıktıya tek bir JSON belgesi yazar | Tamamlandı |
| İlerleme | Uzun komutlar, standart hata bir terminalse ilerlemeyi orada gösterir | Tamamlandı |
| Güvenli iptal | Ctrl+C komutu durdurur; depo önceki durumunda kalır (eski ya da yeni, asla yarım) | Tamamlandı |
| Okunamayan dosyada devam | Okunamayan bir dosya raporlanır ve atlanır; yedek devam eder | Tamamlandı |
| Açık çıkış kodları | Her sonucun kendi çıkış kodu vardır (bölüm 5) | Tamamlandı |

## 2. Deponun seçilmesi

| Kaynak | Örnek | Öncelik |
| --- | --- | --- |
| `--repo` seçeneği | `apeiron --repo /mnt/backup/apeiron log` | 1 |
| `APEIRON_REPO` ortam değişkeni | `export APEIRON_REPO=/mnt/backup/apeiron` | 2 |

İkisi de verilmezse komut bir kullanım hatasıyla durur. Harici diskteki bir depo, o diskteki bir yolda duran sıradan bir depodur.

### 2.1 Şifreli depolar

`apeiron init --encrypt <path>` yeni bir parolayı iki kez sorar ve kurtarma anahtarını bir kez yazdırır ([FORMAT.tr.md](FORMAT.tr.md) §14). Şifreli bir depodaki diğer her komut parolayı ya da kurtarma anahtarını ister:

| Kaynak | Örnek | Öncelik |
| --- | --- | --- |
| `--password-file` seçeneği | `apeiron --password-file ~/.config/apeiron/password log` | 1 |
| `APEIRON_PASSWORD_FILE` ortam değişkeni | `export APEIRON_PASSWORD_FILE=~/.config/apeiron/password` | 2 |
| Terminalde, gösterilmeden sorulur | `Password or recovery key:` | 3 |

Dosyanın ilk satırı paroladır; `init --encrypt` yeni parolayı da oradan okur. Yanlış bir parola ya da kurtarma anahtarı komutu 1 çıkış koduyla ve `key` hata türüyle bitirir. `--json` ile `init`, `recovery_key` alanını ekler.

Bir parolayı değiştirmek için yenisi `apeiron key add` ile eklenir, eskisi `apeiron key list` içinde bulunur (depoyu o açtıysa `(in use)` ile işaretlidir) ve `apeiron key remove <id>` ile silinir. Kimliğin ilk 8 karakteri yeterlidir. Son anahtar silinemez; `key remove`, `gc` gibi depoyu yalnız kullanmak ister.

## 3. Komutlar

| Komut | Ne yapar | Durum |
| --- | --- | --- |
| `apeiron init <path> [--encrypt]` | Boş bir depo oluşturur; `--encrypt` onu bir parolayla şifreler (bölüm 2.1) | Tamamlandı |
| `apeiron backup <folder> [--ref <name>] [--message <text>]` | Klasörü yeni bir snapshot olarak saklar ve ref'i ona taşır (varsayılan ref: `main`) | Tamamlandı |
| `apeiron log [<ref>]` | Bir ref'in snapshot'larını en yeniden başlayarak listeler | Tamamlandı |
| `apeiron ls <snapshot> [<path>]` | Bir snapshot'taki dosya ve klasörleri listeler | Tamamlandı |
| `apeiron diff <snapshot> <snapshot>` | İki snapshot arasında eklenen, silinen ve değişen dosyaları listeler | Tamamlandı |
| `apeiron restore <snapshot> <target> [--path <path>]` | Bir snapshot'ı (ya da içindeki bir yolu) yeni ya da boş bir klasöre yazar | Tamamlandı |
| `apeiron export <snapshot> <file> [--format <format>] [--path <path>]` | Bir snapshot'ı (ya da içindeki bir yolu) yeni bir `.tar`, `.zip` ya da `.7z` dosyasına yazar; `--format` verilmezse biçimi dosya uzantısı belirler | Tamamlandı |
| `apeiron refs` | Tüm ref'leri ve gösterdikleri snapshot'ları listeler | Tamamlandı |
| `apeiron verify` | Her paketi, indeksi ve nesneyi okuyup denetler | Tamamlandı |
| `apeiron repair` | Bayat ref kilitlerini kaldırır, kaybolan reflog satırlarını ekler ve bozuk indeks dosyalarını yeniler | Tamamlandı |
| `apeiron gc [--keep-days <days>] [--dry-run]` | Hiçbir ref'in ve son 90 günün (ya da `--keep-days`) hiçbir reflog satırının ulaşamadığı veriyi siler; `--dry-run` yalnızca raporlar | Tamamlandı |
| `apeiron key list` | Şifreli bir deponun parolalarını ve kurtarma anahtarlarını listeler | Tamamlandı |
| `apeiron key add [--recovery] [--new-password-file <file>]` | Bir parola ekler (iki kez sorulur ya da dosyadan okunur); `--recovery` ile bir kez yazdırılan yeni bir kurtarma anahtarı ekler | Tamamlandı |
| `apeiron key remove <id>` | Bir parolayı ya da kurtarma anahtarını siler; sonuncusu silinemez | Tamamlandı |

Bir `<snapshot>`; ref adı (`main`), tam snapshot kimliği ya da tek bir snapshot'a uyuyorsa kimliğin ilk 8 ya da daha fazla karakteri olarak yazılabilir. `~N` eki ilk ebeveynler üzerinden N adım geri gider: `main~1`, `main`'den önceki snapshot'tır.

## 4. Bir yedeğin sakladıkları

| Öğe | Saklanır | Durum |
| --- | --- | --- |
| Dosyalar, klasörler, sembolik bağlar | Evet | Tamamlandı |
| İzin bitleri ve değişiklik zamanı | Evet | Tamamlandı |
| Linux ve macOS'ta sahip (uid/gid) | Evet | Tamamlandı |
| Genişletilmiş öznitelikler | Aşama 1'de değil | Planlandı |
| Sabit bağlar | Ayrı dosyalar olarak; aralarındaki bağ korunmaz ([FORMAT.tr.md](FORMAT.tr.md) §6.3) | Tamamlandı |
| Aygıt dosyaları, soketler, borular | Hayır; atlanır ve raporlanır | Tamamlandı |

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
| `apeiron` | Herkese açık Rust API'si ([API.tr.md](API.tr.md) §8) | Devam ediyor |
| `apeiron-services` | API'nin kullandığı klasör ↔ snapshot, fark ve dışa aktarma mantığı | Devam ediyor |
| `apeiron-cli` | `apeiron` programı | Devam ediyor |

| İhtiyaç | Önerilen kütüphane | Gerekçe |
| --- | --- | --- |
| Komut satırı ayrıştırma | `clap` | En yaygın Rust ayrıştırıcısı; yardım metnini üretir ve seçenekleri denetler |
| JSON çıktı | `serde_json` | `serde` için standart JSON kütüphanesi; `serde` yapılandırmada zaten kullanılıyor |
| Ctrl+C | `ctrlc` | Küçük ve yaygın; Linux, macOS ve Windows'ta aynı kod |
| Gizli parola sorma | `rpassword` | Küçük ve yaygın; Linux, macOS ve Windows'ta terminalden ekrana yansıtmadan okur |
| tar, zip ve 7z dışa aktarma | `tar`, `zip`, `sevenz-rust2` | Saf Rust; arşivler standart `tar`, `unzip` ve `7z` araçlarıyla izinler ve sembolik bağlar korunarak açılır |

## 8. Açık sorular

| Soru | Öneri |
| --- | --- |
| `backup` için varsayılan ref | `main`; birden fazla klasör için `--ref backup/<name>` |
| `restore` boş olmayan bir klasöre yazabilir mi? | Aşama 1'de hayır; ileride bir `--overwrite` seçeneği buna izin verebilir |
| `export` RAR dosyası yazabilir mi? | Hayır. RAR arşivlerini yalnızca RARLAB'in kendi yazılımı oluşturabilir; açık bir uygulaması yoktur ve ücretsiz unRAR kodu arşiv oluşturmak için kullanılamaz. İçe aktarma için RAR dosyalarını okumak ileride eklenebilir |
| Aynı depoda eşzamanlı komutlar | Karar verildi: komutlar yan yana çalışabilir; `gc` yalnızca tek başına çalışır, aksi hâlde "kilitli" hatasıyla durur ([FORMAT.tr.md](FORMAT.tr.md) §12) |
