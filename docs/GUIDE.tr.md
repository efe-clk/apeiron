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
| Bir yedekte dosyaları ada göre bul | `apeiron search '*.pdf' --in backup/documents` |
| Daha önce silinmiş ya da değişmiş bir dosyayı bul | `apeiron search 'report*' --in backup/documents --history` |
| Bazı sözcükleri içeren metin dosyalarını bul | `apeiron search --text 'invoice 2026' --in backup/documents` |

Bir yedek ref'iyle (`backup/documents`) ya da kimliğinin ilk 8 veya daha fazla karakteriyle (`53373ba8`) anılabilir; `~N` N yedek geri gider (`backup/documents~3`). `diff` çıktısında `A` eklendi, `D` silindi, `M` değişti demektir. Kabuk `*` işaretini açmasın diye bir arama deseni tırnak içinde yazılır; her ölçüt [SEARCH.tr.md](SEARCH.tr.md) içindedir.

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

### 6.3 Bulut depolamada bir kopya (S3)

Amazon S3 ve başka pek çok hizmet (Backblaze B2, Cloudflare R2, Wasabi ve diğerleri) S3 uyumlu depolama sunar. Depo şifreli olmalıdır (bölüm 3.1).

1. Hizmette bir bucket ve bir erişim anahtarı oluşturun. Anahtarı ortam değişkenlerine koyun; örneğin yalnızca sizin okuyabildiğiniz `~/.config/apeiron/env` dosyasına:

   ```bash
   export AWS_ACCESS_KEY_ID=…
   export AWS_SECRET_ACCESS_KEY=…
   ```

2. `apeiron remote add cloud 's3://BUCKET/apeiron?endpoint=https://HIZMET-ADRESI&region=BOLGE'` (Amazon'un kendisi için `endpoint` yazmayın).
3. `source ~/.config/apeiron/env && apeiron push cloud backup/documents`.

Hizmet koşullu yazmayı desteklemelidir; aksi hâlde Apeiron bir ref'i taşımayı reddeder.

### 6.4 İkinci bir cihaz

İkinci bir bilgisayar bu kopyalardan herhangi biriyle başlar:

```bash
apeiron clone /run/media/$USER/DISK/apeiron ~/apeiron   # ya da bir sftp:// veya s3:// adresi
```

Kopyanın parolasını sorar ve `~/apeiron` içinde her yedeği barındıran bir depo oluşturur; kopya artık `origin` olarak bilinir, yani `apeiron --repo ~/apeiron pull origin backup/documents` sonraki yedekleri getirir. `clone` bitmeden durdurulursa yeni klasörü silin ve yeniden çalıştırın.

### 6.5 İki bilgisayarda aynı klasör

Senkron, `~/Documents` gibi bir klasörü bu kopyalardan herhangi biri üzerinden iki ya da daha fazla bilgisayarda eşit tutar ([SYNC.tr.md](SYNC.tr.md)). İlk bilgisayarda:

```bash
apeiron sync ~/Documents disk              # klasörü yedekler ve gönderir
```

İkincisinde, `clone`'dan sonra (bölüm 6.4):

```bash
apeiron --repo ~/apeiron sync --dry-run ~/Documents origin   # neyin değişeceğini gösterir
apeiron --repo ~/apeiron sync ~/Documents origin
```

Bundan sonra `apeiron sync` komutunu iki bilgisayardan birinde çalıştırın ya da bunu arka plan hizmetine bırakın (bölüm 8, `do = "sync"`, `every = "15m"`). Her çalışma önce klasörü yedekler; böylece her hâl geçmişte kalır ve geri yüklenebilir. İki bilgisayar aynı dosyayı farklı değiştirdiyse iki sürüm de tutulur: öteki, yanında `ad (conflict BİLGİSAYAR TARİH SAAT).uzantı` olarak görünür. İkisine bakın, istediğinizi tutun ve kopyayı silin; sonraki senkron bunu da yayar.

### 6.6 Bir Git deposu

Bir Git projesi tüm geçmişiyle Apeiron'da tutulabilir ([GIT.tr.md](GIT.tr.md)):

```bash
apeiron git import ~/code/project           # dallar ve etiketler git/heads/…, git/tags/… olur
apeiron git export ~/restored/project.git   # aynı geçmiş, aynı commit kimlikleriyle
```

Yeni commit'leri saklamak için `git import`'u yeniden çalıştırın; `git export`'tan sonra `git clone ~/restored/project.git` bir çalışma kopyası verir.

### 6.7 Bir dalda bir şey denemek

Bir dal, bir denemeyi `main`'den ayrı tutar ([BRANCH.tr.md](BRANCH.tr.md)):

```bash
apeiron ref create trial                      # main'in olduğu yerden başlayan bir dal
apeiron restore trial ~/trial                 # kendi klasöründe üzerinde çalışın
apeiron backup ~/trial --ref trial            # değişiklikleri kaydedin
apeiron merge trial --dry-run                 # birleştirmenin neyi değiştireceğini görün
apeiron merge trial                           # onu main'e birleştirin
```

Hem `main` hem dal aynı dosyayı değiştirdiyse `main` kendi sürümünü tutar ve dalın sürümü `name (conflict trial).ext` olarak eklenir.

### 6.8 Metin dosyalarını satır satır birleştirmek

Eklenti olmadan, aynı metin dosyasındaki iki değişiklik her zaman bir çatışma kopyası bırakır. Metin eklentisi farklı satırlardaki değişiklikleri birleştirir ([TEXT.tr.md](TEXT.tr.md)). Bir kez derlenir ve kullanıcınız için kurulur:

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2 --manifest-path plugins/text/Cargo.toml
apeiron plugin add plugins/text/target/wasm32-wasip2/release/apeiron_text.wasm
apeiron diff main~1 main --text               # hangi satırların değiştiğini görün
```

Klasörü senkronlayan her bilgisayara kurun; kurulu olmayan bir bilgisayar bunun yerine çatışma kopyası oluşturur, bu da hiçbir şey kaybettirmez.

### 6.9 Komut satırı olmadan

Masaüstü uygulaması, 3. ile 6. bölümler arasında çoğu kişinin ihtiyaç duyduğu her şeyi bir pencerede yapar: yedek klasörünü açmak ya da oluşturmak, yedeklemek, sürümlere bakmak, geri yüklemek ve denetlemek ([DESKTOP.tr.md](DESKTOP.tr.md)). Linux'ta önce WebKitGTK dosyalarını ister (Debian ve Ubuntu'da `sudo apt install libwebkit2gtk-4.1-dev`, Arch'ta `sudo pacman -S webkit2gtk-4.1`):

```bash
cargo install --locked --path crates/apeiron-desktop
apeiron-desktop
```

Masaüstü uygulamasının derlenemediği yerde `apeiron ui` aynı sayfaları tarayıcınızda gösterir; durdurmak için terminali kapatın ya da Ctrl+C'ye basın.

## 7. Düzen

| Ne sıklıkla | Komut | Neden |
| --- | --- | --- |
| Günlük (ya da otomatik, bölüm 8) | Her klasör için `apeiron backup …` | Değişenlerin yeni bir snapshot'ı |
| Her yedekten sonra ya da haftalık | Her ref için `apeiron push disk …` (bölüm 6.1) | Bilgisayardan bağımsız ikinci bir kopya |
| Haftalık | `apeiron verify` | Saklanan her baytı okur ve denetler. `OK: …` bozulma yok demektir; çıkış kodu 4 bozuk dosyaları listeler |
| Aylık | `apeiron gc` | Hiçbir ref'in ulaşmadığı ve 90 günden eski veriyi siler. Önce `apeiron gc --dry-run` ile deneyin |
| Aylık | Birkaç dosyayı yeni bir klasöre geri yükleyip açın | Bir yedek ancak geri yüklenerek kanıtlanır |
| Bir uyarı söylediğinde | `apeiron repair` | Bayat kilitleri kaldırır, reflog'ları tamamlar ve bozuk indeks dosyalarını yeniden kurar |

## 8. Otomatik yedekler (arka plan hizmeti)

Arka plan hizmeti; yedekleri, push'ları ve denetimleri bir zamanlamaya göre çalıştırır ([DAEMON.tr.md](DAEMON.tr.md)). Bilgisayar kapalı ya da uykudayken kaçırılan bir çalışma, ilk fırsatta bir kez yapılır.

1. İşleri `~/.config/apeiron/jobs.toml` dosyasına yazın (macOS: `~/Library/Application Support/apeiron/jobs.toml`, Windows: `%APPDATA%\apeiron\jobs.toml`):

   ```toml
   [job.documents]
   repo = "/run/media/me/disk/apeiron"
   password_file = "~/.config/apeiron/password"   # yalnızca şifreli bir depo için
   do = "backup"
   folder = "~/Documents"
   ref = "backup/documents"
   every = "day"
   at = "12:30"
   push = ["nas"]                                 # isteğe bağlı: uzak depolar (bölüm 6)

   [job.check]
   repo = "/run/media/me/disk/apeiron"
   password_file = "~/.config/apeiron/password"
   do = "restore-test"
   ref = "backup/documents"
   every = "week"
   ```

2. Bir kez deneyin: `apeiron daemon once`, zamanı gelmiş her işi çalıştırır ve sonuçları yazdırır.
3. Oturum açtığınızda `apeiron daemon run` komutunu sistemin başlatmasını sağlayın:

   | Sistem | Nasıl |
   | --- | --- |
   | Linux | `~/.config/systemd/user/apeiron.service` dosyasını oluşturun (aşağıda), sonra `systemctl --user enable --now apeiron` |
   | macOS | `~/Library/LaunchAgents/org.apeiron.daemon.plist` dosyasını oluşturun (aşağıda), sonra `launchctl load -w ~/Library/LaunchAgents/org.apeiron.daemon.plist` |
   | Windows | `schtasks /Create /SC ONLOGON /TN Apeiron /TR "%USERPROFILE%\.cargo\bin\apeiron.exe daemon run"` |

   ```ini
   [Unit]
   Description=Apeiron background service

   [Service]
   ExecStart=%h/.cargo/bin/apeiron daemon run
   Restart=on-failure

   [Install]
   WantedBy=default.target
   ```

   ```xml
   <?xml version="1.0" encoding="UTF-8"?>
   <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
   <plist version="1.0">
   <dict>
     <key>Label</key><string>org.apeiron.daemon</string>
     <key>ProgramArguments</key>
     <array><string>/Users/me/.cargo/bin/apeiron</string><string>daemon</string><string>run</string></array>
     <key>RunAtLoad</key><true/>
     <key>KeepAlive</key><true/>
   </dict>
   </plist>
   ```

4. Ara sıra sonuçlara bakın: `apeiron daemon status` her işin son ve sıradaki çalışmasını listeler; çıkış kodu 4 bir işin başarısız olduğu anlamına gelir. Ayrıntılar, durum dosyasının yanındaki `daemon.log` içindedir (Linux: `~/.local/state/apeiron`).

Kimsenin oturum açmadığı bir ev sunucusunda `loginctl enable-linger <user>`, Linux hizmetinin oturum açılmadan çalışmasını sağlar. Harici disk takılı değilse iş "no config file" hatasıyla başarısız olur ve bir saat sonra yeniden denenir.

## 9. Bir aylık deneme

Aşama 1'in çıkış kriteri, kendi verinizi bir ay boyunca yalnızca Apeiron ile yedekleyip geri yüklemektir. Kısa notlar tutun; Aşama 2'de neyin önce düzeltileceğini onlar belirler.

| Hafta | Yapın | Not edin |
| --- | --- | --- |
| 1 | Kurulum, depoyu oluşturma, ilk yedekler, arka plan hizmetini kurma | Bu rehberde anlaşılmayan her şey; ilk yedeğin ne kadar sürdüğü |
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
