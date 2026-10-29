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

### 3.1 Şifreli depo

Disk kaybolabilir ya da çalınabilirse veya daha sonra bir sunucuya kopyalanacaksa depoyu bunun yerine şifreli oluşturun:

```bash
apeiron init --encrypt /run/media/<user>/<disk>/apeiron
```

Komut bir parolayı iki kez sorar ve bir **kurtarma anahtarını** bir kez yazdırır (dörder karakterlik 13 grup). Kurtarma anahtarını kâğıda yazın ve bilgisayardan ve diskten ayrı bir yerde saklayın. Depoyu parola da kurtarma anahtarı da açar; ikisi birden kaybolursa veriyi hiç kimse kurtaramaz. Şifreleme yalnızca depo oluşturulurken seçilebilir.

Sonraki her komut `Password or recovery key:` diye sorar. Otomatik yedek (bölüm 8) için parolayı yalnızca sizin okuyabildiğiniz bir dosyaya yazın ve Apeiron'a yerini söyleyin:

```bash
mkdir -p ~/.config/apeiron
printf '%s\n' 'parolanız' > ~/.config/apeiron/password
chmod 600 ~/.config/apeiron/password
export APEIRON_PASSWORD_FILE=~/.config/apeiron/password
```

Anahtarı olmayan ve diski elinde tutan biri yalnızca veri dosyalarının sayısını ve boyutunu ve ref adlarını (`backup/documents`) görür; dosya adlarını ve içeriği göremez ([FORMAT.tr.md](FORMAT.tr.md) §14.5).

### 3.2 Parolayı değiştirme

Önce yeni parolayı ekleyin, sonra eskisini silin:

```bash
apeiron key add                 # önce mevcut parolayı, sonra yenisini iki kez sorar
apeiron key list                # eski parola (in use) ile işaretlidir
apeiron key remove <id>         # eski parolanın kimliğinin ilk 8 karakteri
```

Parola dosyası kullanılıyorsa `key add`'den sonra yeni parolayı dosyaya yazın. `apeiron key add --recovery` yeni bir kurtarma anahtarı oluşturur (örneğin eski kâğıt kaybolduğunda); eski kurtarma anahtarı da aynı yolla silinir. Son anahtar silinemez ve `key remove` hiçbir yedek çalışmıyorken yapılmalıdır.

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

### 6.1 Harici diskte ikinci bir kopya

Aynı bilgisayardaki bir yedek, bilgisayarla birlikte kaybolur. Harici bir diskte ikinci bir kopya tutun:

```bash
apeiron remote add disk /run/media/$USER/DISK/apeiron   # bir kez; diskin klasörünü yazın
apeiron push disk backup/documents                      # her yedekten sonra
```

İlk push her şeyi kopyalar; sonrakiler yalnızca yeni olanı. Diskteki klasör aynı parolaya sahip eksiksiz bir depodur: bilgisayar kaybolursa doğrudan ondan geri yükleyin: `apeiron --repo /run/media/$USER/DISK/apeiron restore backup/documents ~/restored`. Yarıda kalan bir push yeniden çalıştırılır.

### 6.2 Ev sunucusunda bir kopya (SFTP)

Evde ya da başka bir yerde `ssh` ile ulaştığınız bir sunucu da bir kopya tutabilir. Depo şifreli olmalıdır (bölüm 3.1): bilgisayardan yalnızca şifreli veri çıkar.

1. `ssh user@nas.local` komutunun parola yazmadan çalıştığından emin olun; SSH ajanında bir anahtar ya da parolasız `~/.ssh/id_ed25519` yeterlidir. İlk `ssh` sizden sunucunun anahtarını denetlemenizi ister; Apeiron yalnızca bu şekilde denetlediğiniz sunuculara güvenir.
2. `apeiron remote add nas sftp://user@nas.local/srv/apeiron` (başka bir port için sunucu adından sonra `:2222` ekleyin).
3. Diskteki gibi `apeiron push nas backup/documents`.

## 7. Düzen

| Ne sıklıkla | Komut | Neden |
| --- | --- | --- |
| Günlük (ya da otomatik, bölüm 8) | Her klasör için `apeiron backup …` | Değişenlerin yeni bir snapshot'ı |
| Her yedekten sonra ya da haftalık | Her ref için `apeiron push disk …` (bölüm 6.1) | Bilgisayardan bağımsız ikinci bir kopya |
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
# Only for an encrypted repository (section 3.1):
Environment=APEIRON_PASSWORD_FILE=%h/.config/apeiron/password
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
| `wrong password or recovery key` | Parolayı yeniden yazın; kurtarma anahtarı da tireli ya da tiresiz çalışır. Parola dosyası kullanıyorsanız ilk satırının parola olduğunu denetleyin |
| `already exists and is not an empty folder` | `restore` ve `export` asla üzerine yazmaz; yeni bir hedef seçin |
| Çıkış kodu 130 | Komut Ctrl+C ile iptal edildi; yarım kalmış hiçbir şey görünmez |

Çıkış kodları [CLI.tr.md](CLI.tr.md) §5'te listelenir. Sorunları GitHub issue olarak, güvenlik sorunlarını ise [SECURITY.tr.md](../SECURITY.tr.md)'de anlatıldığı gibi bildirin.
