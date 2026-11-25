> **English:** [CONSISTENT.md](CONSISTENT.md)

# Uygulama Tutarlı Yedekler

> **Durum: Tamamlandı.** Çalışan bir veritabanının ya da yoğun bir diskin yedeği, dosyaları bir değişikliğin ortasında yakalayabilir. `apeiron backup --source` bunun yerine bir veritabanı dökümünü, bir dosya sisteminin salt okunur anlık görüntüsünü ya da bir konteyner biriminin içeriğini yedekler; her biri sistemin kendi programıyla tek bir anda yapılır. Arkasındaki karar ADR-059'dur ([DECISIONS.tr.md](DECISIONS.tr.md)).

## 1. Komut

```bash
apeiron backup --source postgres:shop                 # postgres/shop içine
apeiron backup --source btrfs:/srv/photos --ref photos
apeiron backup --source docker:pgdata --pause
```

`apeiron backup --source <kind>:<target> [--ref <name>] [--pause]` bir klasörün yerini alır. `--ref` olmadan snapshot `<kind>/<name>`'e gider, örneğin `postgres/shop`. Arka plan hizmetinin bir işi aynısını `folder` yerine `source = "<kind>:<target>"` ve `pause = true` ile yapar ([DAEMON.tr.md](DAEMON.tr.md) §3); bir kiracının işleri yapamaz, çünkü sunucuda programlar çalıştırırlardı. Yedek için yapılan her şey, yedek başarısız olsa da ardından silinir.

## 2. Kaynaklar

| Kaynak | Ne yedeklenir | Yapan | Gerekenler | Denendi |
| --- | --- | --- | --- | --- |
| `postgres:<veritabanı ya da bağlantı>` | Düz bir SQL dökümü, `<veritabanı>.sql` | Veritabanının tek bir anlık görüntüsünü okuyan `pg_dump` | `PGPASSWORD`'den ya da `~/.pgpass`'ten parola; en az sunucu kadar yeni bir `pg_dump` | Evet, PostgreSQL 17 |
| `mysql:<veritabanı>` | Düz bir SQL dökümü, `<veritabanı>.sql` | `--single-transaction` ile `mysqldump` ya da `mariadb-dump` | `~/.my.cnf`'ten ya da `MYSQL_PWD`'den parola | Hayır |
| `btrfs:<alt birim>` | Alt birimin yanında yapılan salt okunur bir anlık görüntü | `btrfs subvolume snapshot -r` | Bir alt birim; anlık görüntüyü silmek root ister, yoksa yazılabilir yapılıp sahibince silinir | Evet |
| `zfs:<veri kümesi>` | `.zfs/snapshot` üzerinden okunan anlık görüntü | `zfs snapshot`, `zfs destroy` | Veri kümesinde haklar (`zfs allow`) ya da root | Hayır |
| `lvm:<grup>/<birim>` | Kaynağın onda biri boyutunda, salt okunur bağlanan bir anlık görüntü birimi | `lvcreate --snapshot`, `mount`, `lvremove` | Root ve birim grubunda yer | Hayır |
| `docker:<birim>` | İçe aktarılan bir tar arşivi olarak birim (CLI.tr.md §4.2) | Birimi salt okunur bağlayan bir `busybox` konteyneri | Docker; `--pause`, birim okunurken onu kullanan konteynerleri duraklatır | Evet |
| `kubernetes:<ad alanı>/<talep>` | İçe aktarılan bir tar arşivi olarak kalıcı birim | Talebi salt okunur bağlayan, sonra silinen yardımcı bir pod | Pod çalıştırma hakkı olan `kubectl`; tek bir düğüme ait bir talep yalnızca o düğüm pod'u çalıştırabiliyorsa okunur | Hayır |
| `vss:<klasör>` | Sürücüsünün bir Volume Shadow Copy'sindeki klasör | PowerShell üzerinden Windows'un gölge kopya hizmeti | Yönetici hakları | Hayır |

Düz SQL dökümü günden güne az değişir; bu yüzden chunk'ları tekilleştirilir (FORMAT.tr.md §7); sıkıştırılmış bir döküm her seferinde yeni olurdu. Dökümler ve arşivler deponun `tmp/` klasörüne yazılır; orada birinin sığacağı kadar yer gerekir.

## 3. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Bir veritabanına geri yüklemek | Dökümü geri yükleyip, her dökümde olduğu gibi `psql` ya da `mysql` ile yükleyin |
| MongoDB gibi diğer veritabanları | Daha sonra; bir iş, bir döküm programının yazdığı klasörü yedekleyebilir |
| "Hayır" işaretli kaynakları gerçek sistemlerde denemek | Her birinin ilk kullanıcılarıyla |

## 4. Crate'ler

| Crate | Görev | Durum |
| --- | --- | --- |
| `apeiron-daemon` | `sources`: her kaynağı hazırlamak ve silmek | Tamamlandı |
| `apeiron-cli` | `apeiron backup --source` | Tamamlandı |
