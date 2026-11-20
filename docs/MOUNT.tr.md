> **English:** [MOUNT.md](MOUNT.md)

# Bir Depoyu Bağlamak

> **Durum: Linux ve macOS'ta tamamlandı.** `apeiron mount`, bir deponun her snapshot'ını salt okunur bir klasör olarak gösterir; böylece dosyaları önce geri yüklemeden açılabilir, `grep` ile aranabilir ya da `diff` ile karşılaştırılabilir. Arkasındaki karar ADR-056'dır ([DECISIONS.tr.md](DECISIONS.tr.md)).

## 1. Komut

```text
$ mkdir ~/backups
$ apeiron --repo /run/media/user/disk/apeiron mount ~/backups
Mounted at /home/user/backups; press Ctrl+C or unmount it to stop
```

`apeiron mount <folder>`, depoyu boş bir klasöre bağlar ve Ctrl+C'ye ya da klasör dışarıdan çözülene kadar sunar (Linux'ta `fusermount3 -u <folder>`, macOS'ta `umount <folder>`). Şifreli bir depo, her komutta olduğu gibi önce parolasını sorar.

## 2. Ne gösterir

```text
~/backups/backup/documents/latest/…
~/backups/backup/documents/2026-11-20T103000Z_3f9ae1c2/…
~/backups/sync/photos/latest/…
```

| Yol | İçerik |
| --- | --- |
| Kök ve klasörleri | Her ref adının her parçası için bir klasör: `backup/documents` ref'i `backup/documents` klasörüdür |
| Bir ref'in klasörü | `latest`, ref'in şimdi gösterdiği snapshot, ve geçmişindeki (ilk ebeveyne göre) her snapshot için UTC zamanı ve kimliğinin başıyla adlandırılmış bir klasör |
| Bir snapshot'ın klasörü | Dosyaları, klasörleri ve sembolik bağlantıları; kaydedilmiş boyutları, zamanları ve izin bitleriyle |

`apeiron/` ve `folder/` altındaki ref'ler Apeiron'un kendisine aittir ve gösterilmez. Depo bağlıyken oluşturulan ya da taşınan bir ref bir saniye içinde görünür. Her şey salt okunurdur: yazmak, yeniden adlandırmak ve silmek "salt okunur dosya sistemi" ile başarısız olur ve izin bitleri yazma bitleri olmadan gösterilir. Dosyalar ve klasörler onlara bakan kişiye aittir; çünkü yedek anında kaydedilen sahipler bu bilgisayarda olmayabilir.

## 3. Nasıl çalışır

Bağlama yalnızca herkese açık API'yi kullanır ([API.tr.md](API.tr.md)): ref'ler, her ref'in geçmişi ve her klasörün listesi `refs`, `log` ve `list`'ten gelir; bir dosya `FileReader::read_at` ile herhangi bir konumdan okunur, böylece büyük bir dosyanın ortasından birkaç bayt okuyan bir program yalnızca gereken chunk'ları getirir. Her chunk, geri yüklemede olduğu gibi okunurken kimliğine karşı denetlenir. Bir snapshot asla değişmez; bu yüzden listeleri depo bağlı kaldıkça tutulur ve sistem sayfalarını bir saat tutabilir; ref klasörleri bir saniye sonra yeniden okunur.

## 4. Sistemler

| Sistem | Gerekenler | Durum |
| --- | --- | --- |
| Linux | FUSE 3: `fusermount3`'ü veren `fuse3` paketi | Tamamlandı |
| macOS | [macFUSE](https://macfuse.github.io) | Yalnızca Linux'ta derlendi ve sınandı; macOS'ta kullanıcının macFUSE kurması gerekir |
| Windows | — | Yazılmadı: Windows'ta FUSE yok ve Projected File System ayrı bir katman ister (§5) |

## 5. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Projected File System ya da WinFsp ile Windows | Daha sonra; o zamana kadar `apeiron restore` ve masaüstü arayüzünün Göz at sayfası |
| Bağlı bir snapshot'a yazmak | Planlanmadı: snapshot'lar asla değişmez; bunun yerine bir klasörü yedekleyin |
| Yerel depo olmadan bir uzak depoyu bağlamak | Daha sonra; önce onu clone ya da pull edin |

## 6. Crate'ler ve kütüphaneler

| Crate | Görev | Durum |
| --- | --- | --- |
| `apeiron-mount` | Bağlamanın klasörleri ve FUSE dosya sistemi | Tamamlandı |
| `apeiron-cli` | `apeiron mount` | Tamamlandı |

| İhtiyaç | Kütüphane | Gerekçe |
| --- | --- | --- |
| FUSE | libfuse olmadan `fuser` | Saf Rust; FUSE protokolünü kendisi konuşur ve bağlamak için yalnızca `fusermount3` ister, böylece libfuse'a karşı hiçbir şey derlenmez |
