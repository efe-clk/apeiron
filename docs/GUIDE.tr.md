> **English:** [GUIDE.md](GUIDE.md)

# Apeiron Kullanım Rehberi

> **Durum:** Aşama 1 denemesi için ([ROADMAP.tr.md](ROADMAP.tr.md) §3). Apeiron'un henüz bir sürümü yok; kaynak koddan derlenir. Aşağıdaki tüm komutlar bugün çalışır. Tam komut başvurusu [CLI.tr.md](CLI.tr.md) içindedir.

Bu rehber kurulumdan harici diske günlük yedeğe, dosyaları geri yüklemeye ve bir aylık denemenin düzenine kadar yol gösterir.

## 1. Başlamadan önce

| Konu | Öneri |
| --- | --- |
| Eski yedeğinizi koruyun | Apeiron erken geliştirme aşamasındadır. Deneme boyunca var olan yedeğinize güvenlik ağı olarak dokunmayın; silmeyin |
| Depo nerede durur | Koruduğu veriden farklı bir diskte, örneğin harici bir diskte. Depo sıradan bir klasördür |
| Platformlar | Bu rehberin ana hedefi Linux ve macOS'tur; Windows da çalışır, ama bölüm 8'deki otomatik yedek yalnızca Linux içindir |

## 2. Kurulum

### 2.1 Araçlar

Apeiron Rust ile yazılmıştır. Rust araçları, Git ve bir C derleyicisi gerekir.

| Sistem | Komut |
| --- | --- |
| Arch Linux, CachyOS, Manjaro | `sudo pacman -S rustup git gcc` ardından `rustup default stable` |
| Debian, Ubuntu | `sudo apt install git build-essential` ardından Rust'ı [rustup.rs](https://rustup.rs) üzerinden kurun |
| macOS | `xcode-select --install` ardından Rust'ı [rustup.rs](https://rustup.rs) üzerinden kurun |
| Windows | Git'i, Visual Studio C++ derleme araçlarını ve Rust'ı [rustup.rs](https://rustup.rs) üzerinden kurun |

### 2.2 Derleme ve kurulum

```bash
git clone https://github.com/efe-clk/apeiron.git
cd apeiron
cargo install --path crates/apeiron-cli
apeiron --version
```

`cargo install` programı derler ve `~/.cargo/bin/apeiron` konumuna kopyalar. `apeiron --version` "command not found" derse `~/.cargo/bin`'i `PATH`'e ekleyin (rustup genellikle bunu yapar; önce yeni bir terminal açın).

### 2.3 Güncelleme

```bash
cd apeiron
git pull
cargo install --path crates/apeiron-cli
```

## 3. Depoyu oluşturma

Harici diski takın ve nereye bağlandığını bulun (çoğu Linux masaüstünde `/run/media/<user>/<disk>`).

```bash
apeiron init /run/media/<user>/<disk>/apeiron
```

Yolu her seferinde yazmamak için bu satırı `~/.zshrc`'ye (ya da `~/.bashrc`'ye) ekleyin ve yeni bir terminal açın:

```bash
export APEIRON_REPO=/run/media/<user>/<disk>/apeiron
```

Her komut ayrıca öncelikli olan `--repo <path>` seçeneğini kabul eder.

## 4. Yedekleme

Her klasöre kendi ref'ini verin; böylece her birinin kendi geçmişi olur:

```bash
apeiron backup ~/Documents --ref backup/documents -m "first backup"
apeiron backup ~/Pictures  --ref backup/pictures
```

| Gördüğünüz | Anlamı |
| --- | --- |
| `Snapshot 53373ba88b2e (backup/documents): 120 files, 14 folders, 1.2 GiB read, 3.4 MiB new` | Yedek tamamlandı. "new", depoda henüz olmayan veridir; değişmeyen dosyalar asla iki kez saklanmaz |
| `warning: <path>: …` ve çıkış kodu 3 | Yedek tamamlandı, ama bazı girdiler atlandı (okunamayan dosyalar, soketler) ya da okunurken değişti. Uyarıları okuyun |
| Yedek sırasında ilerleme satırı | Tamamlanan dosya ve baytlar ve o anki dosya. Ctrl+C güvenle durdurur: ref taşınmaz ve yarım yazılmış hiçbir şey görünmez |

Ref adlarında yalnızca küçük harf, rakam, `.`, `_`, `-` ve `/` kullanılır (`Belgelerim` değil, `backup/documents`).

## 5. Yedeklere bakma

| İş | Komut |
| --- | --- |
| Tüm ref'leri listele | `apeiron refs` |
| Bir klasörün geçmişi, en yeniden başlayarak | `apeiron log backup/documents` |
| Bir yedeğin içindeki dosyalar | `apeiron ls backup/documents` ya da `apeiron ls backup/documents some/folder` |
| Son iki yedek arasında ne değişti | `apeiron diff backup/documents~1 backup/documents` |

Bir yedek ref'iyle (`backup/documents`) ya da kimliğinin ilk 8 veya daha fazla karakteriyle (`53373ba8`) anılabilir; `~N` N yedek geri gider (`backup/documents~3`). `diff` çıktısında `A` eklendi, `D` silindi, `M` değişti demektir.

## 6. Geri yükleme

Apeiron var olan dosyaların üzerine asla yazmaz. Hedef klasör var olmamalı ya da boş olmalıdır.

| İş | Komut |
| --- | --- |
| Son yedekteki her şey | `apeiron restore backup/documents ~/restored` |
| Daha eski bir yedekten | `apeiron restore backup/documents~2 ~/restored` |
| Tek bir klasör | `apeiron restore backup/documents ~/restored --path projects/thesis` |
| Tek bir dosya | `apeiron restore backup/documents ~/restored --path notes/todo.txt` |

Ardından ihtiyacınız olanı `~/restored`'dan yerine kopyalayın. İzinler ve zamanlar geri yüklenir; sahipler yalnızca onları ayarlama hakkınız varsa.

Bir yedeği Apeiron'u olmayan birine vermek için onu bir arşive yazın: `apeiron export backup/documents ~/documents.zip` (`.tar` ve `.7z` de çalışır).

## 7. Düzen

| Ne sıklıkla | Komut | Neden |
| --- | --- | --- |
| Günlük (ya da otomatik, bölüm 8) | Her klasör için `apeiron backup …` | Değişenlerin yeni bir snapshot'ı |
| Haftalık | `apeiron verify` | Saklanan her baytı okur ve denetler. `OK: …` bozulma yok demektir; çıkış kodu 4 bozuk dosyaları listeler |
| Aylık | `apeiron gc` | Hiçbir ref'in ulaşmadığı ve 90 günden eski veriyi siler. Önce `apeiron gc --dry-run` ile deneyin |
| Aylık | Birkaç dosyayı yeni bir klasöre geri yükleyip açın | Bir yedek ancak geri yüklenerek kanıtlanır |
| Bir uyarı söylediğinde | `apeiron repair` | Bayat kilitleri kaldırır, reflog'ları tamamlar ve bozuk indeks dosyalarını yeniden kurar |

## 8. Otomatik günlük yedek (Linux)

Bir systemd kullanıcı zamanlayıcısı yedeği her gün çalıştırır; bilgisayar planlanan saatte kapalıysa açıldıktan sonra da çalıştırır. `~/.config/systemd/user/apeiron-backup.service` dosyasını oluşturun:

```ini
[Unit]
Description=Apeiron backup of Documents

[Service]
Type=oneshot
Environment=APEIRON_REPO=/run/media/<user>/<disk>/apeiron
ExecStart=%h/.cargo/bin/apeiron backup %h/Documents --ref backup/documents
# Exit code 3 means "done with warnings".
SuccessExitStatus=3
```

ve `~/.config/systemd/user/apeiron-backup.timer` dosyasını:

```ini
[Unit]
Description=Daily Apeiron backup

[Timer]
OnCalendar=daily
Persistent=true

[Install]
WantedBy=timers.target
```

Ardından etkinleştirin ve sonuçlara bakın:

```bash
systemctl --user daemon-reload
systemctl --user enable --now apeiron-backup.timer
systemctl --user list-timers apeiron-backup.timer
journalctl --user -u apeiron-backup.service
```

Harici disk takılı değilse o günün çalışması "no config file" hatasıyla başarısız olur ve sonraki çalışma yeniden dener. Yerleşik bir zamanlayıcı Aşama 2'de arka plan hizmetiyle gelecek.

## 9. Bir aylık deneme

Aşama 1'in çıkış kriteri, kendi verinizi bir ay boyunca yalnızca Apeiron ile yedekleyip geri yüklemektir. Kısa notlar tutun; Aşama 2'de neyin önce düzeltileceğini onlar belirler.

| Hafta | Yapın | Not edin |
| --- | --- | --- |
| 1 | Kurulum, depoyu oluşturma, ilk yedekler, zamanlayıcıyı kurma | Bu rehberde anlaşılmayan her şey; ilk yedeğin ne kadar sürdüğü |
| 2 | Günlük yedekler; `apeiron verify`; bir klasörü geri yükleyip karşılaştırma | Gördüğünüz uyarılar; geri yüklenen dosyaların açılıp açılmadığı |
| 3 | Silinmiş bir dosyayı daha eski bir yedekten (`~N`) geri yükleme; `apeiron gc --dry-run` | Doğru yedeği bulmanın ne kadar kolay olduğu |
| 4 | Her şeyi yeni bir klasöre geri yükleyip özgün hâliyle karşılaştırma; `apeiron gc` | Eksik ya da hatalı dosyalar, hata mesajları, deponun boyutu |

## 10. Bir şey ters giderse

| Mesaj | Anlamı ve yapılacak |
| --- | --- |
| `no repository given` | `APEIRON_REPO`'yu ayarlayın (bölüm 3) ya da `--repo` verin |
| `invalid repository … no config file` | Yol yanlış ya da harici disk bağlı değil |
| `the repository is locked by another program` | `gc` yalnızca depoyu başka hiçbir şey kullanmıyorken çalışır ve `gc` sürerken diğer komutlar başlayamaz. Bekleyip yeniden deneyin |
| `warning: damaged file …` | Ayrıntı için `apeiron verify`, bozuk indeks dosyalarını yeniden kurmak için `apeiron repair` çalıştırın. Bozuk paketler henüz onarılamaz (Aşama 2 başka hedeflerde kopyalar getirir) |
| `already exists and is not an empty folder` | `restore` ve `export` asla üzerine yazmaz; yeni bir hedef seçin |
| Çıkış kodu 130 | Komut Ctrl+C ile iptal edildi; yarım kalmış hiçbir şey görünmez |

Çıkış kodları [CLI.tr.md](CLI.tr.md) §5'te listelenir. Sorunları GitHub issue olarak, güvenlik sorunlarını ise [SECURITY.tr.md](../SECURITY.tr.md)'de anlatıldığı gibi bildirin.
