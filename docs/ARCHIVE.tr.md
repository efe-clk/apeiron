> **English:** [ARCHIVE.md](ARCHIVE.md)

# Değiştirilemez Arşiv

> **Durum: Tamamlandı.** Aşama 5'in 4. işinin tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §7): bir kurumun saklaması gereken süre dolmadan kimsenin değiştiremeyeceği ya da silemeyeceği bir arşiv olarak tutulan yedekler; ne bir hatayla, ne bir admin tarafından, ne de bir admin'in hesabını ele geçirmiş bir fidye yazılımı tarafından. Arkasındaki karar ADR-034'tür ([DECISIONS.tr.md](DECISIONS.tr.md)).

Yasalar ve sözleşmeler çoğu zaman kayıtların ne kadar saklanacağını söyler: faturalar on yıl, sağlık kayıtları daha uzun. Bir denetçi de gösterilenin saklananla aynı olduğuna dair kanıt ister. Fidye yazılımı ise önce yedekleri arar ve siler.

## 1. İki katman

| Katman | Neye karşı korur | Nasıl |
| --- | --- | --- |
| Saklama kuralları (§2) | Hatalar, Apeiron'un kullanıcıları ve admin'leri, API'sini kullanan programlar | Korunan bir ref'i geri almayı ya da zamanından önce silmeyi Apeiron'un kendisi reddeder |
| Değiştirilemeyen depolama (§4) | Dosyalara ya da depolama hesabına erişimi olan biri | Object Lock'lu bir S3 kovası, kim isterse istesin saklanan veriyi silmeyi ya da üzerine yazmayı reddeder |

İlk katman her yerde çalışır ve hiçbir şeye mal olmaz; ikincisi bunu destekleyen bir depolama ister. İkisinin altında tutulan bir kopya, örneğin Object Lock'lu bir kovadaki bir replika ([REPLICATION.tr.md](REPLICATION.tr.md)), zamanından önce kimse tarafından değiştirilemez.

## 2. Saklama kuralları

Bir kural, bir ad ya da ardından `*` gelen bir önek olarak ref'leri ve bir gün sayısını adlandırır:

```text
apeiron retention add "archive/*" --days 3650
apeiron backup /srv/invoices --ref archive/invoices
```

Bir kuralın eşleştiği her ref için ([FORMAT.tr.md](FORMAT.tr.md) §16):

| Değişiklik | İzinli |
| --- | --- |
| Ref'e yeni bir yedek ya da ona bir birleştirme | Evet: geçmiş büyür |
| Ref'i daha eski bir snapshot'a ya da başka bir geçmişe geri almak | Asla |
| Ref'i silmek | Yalnızca son değişikliğinden bu yana gün sayısı geçtiğinde |
| Eşleşen yeni bir ref oluşturmak | Evet |

Korunan bir ref yalnızca büyüdüğü için gösterdiği her snapshot ulaşılabilir kalır ve çöp toplama hepsini tutar. Bir kural CLI, API ya da web arayüzü üzerinden kaldırılamaz ya da kısaltılamaz; yanına daha uzun bir kural eklenebilir ve en uzunu geçerlidir. Bir ref'le birkaç kural eşleştiğinde en uzunu uygulanır. Reddedilen bir değişiklik `conflict` ile başarısız olur ve kaç gün kaldığını söyler.

Kurallar depoyla birlikte gider: push onları uzak depoya yazar, pull ya da klon geri getirir; böylece bir replika aynı ref'leri korur (FORMAT.tr.md §16.3).

## 3. Saklananı kanıtlamak

Kanıt için her deponun sahip olduğundan fazlası gerekmez: her nesne özetiyle adlandırılır ve her snapshot kendi tree'sini ve ebeveynlerini adlandırır ([FORMAT.tr.md](FORMAT.tr.md) §4); bu yüzden bir snapshot'ın kimliği, altındaki her dosyanın her baytını ve ondan önceki tüm geçmişi sabitler. `apeiron verify` saklanan her baytı okur ve bu özetlerle denetler. Oluşturulduğu anda bir snapshot kimliği verilmiş bir denetçi, arşivin hâlâ tam olarak onu tuttuğunu denetleyebilir.

## 4. Değiştirilemeyen depolama

Apeiron'un kuralları Apeiron'u bağlar. Deponun klasörüne ya da depolama hesabına yazabilen biri dosyaları yine de silebilir. Uyumluluk (compliance) kipinde Object Lock'lu bir S3 kovası bu boşluğu kapatır: saklama süresi bitmeden saklanan herhangi bir sürümü silmeyi ya da üzerine yazmayı, hesabın sahibi için bile S3'ün kendisi reddeder.

1. Kovayı, Object Lock ve uyumluluk kipinde, en az §2'deki kurallar kadar uzun bir varsayılan saklama süresiyle oluşturun.
2. Onu şifreli bir deponun uzak deposu olarak ekleyin: `apeiron remote add vault "s3://bucket/apeiron?endpoint=…&region=…"`.
3. `apeiron push --all vault`, zamanlanmış bir iş ya da `apeiron web --replicate vault` ile güncel tutun ([REPLICATION.tr.md](REPLICATION.tr.md)).

Apeiron'un S3'e yazdığı her nesne, Object Lock'lu kovaların istediği ve veriyi yolda koruyan bir SHA-256 sağlama toplamı taşır. Paketler ve indeks dosyaları yalnızca yazılır, asla değiştirilmez; bu yüzden her biri ilk saklandığı gibi tutulur. Taşınan bir ref ya da bir push'un sildiği bir dosya yeni bir sürüm ya da bir silme işareti olur: önceki sürümler kilitli kalır ve depolamanın admin'i onları geri getirebilir.

## 5. JSON API'de ve CLI'da

| Nerede | Ne |
| --- | --- |
| `apeiron retention add <refs> --days <n>` | Bir kural ekler; geri alınamaz |
| `apeiron retention list` | Kuralları listeler |
| `retention`, `add_retention` (API.tr.md §8) | Programlar için aynısı |
| Web ve masaüstü arayüzlerinin Saklama sayfası | Kuralları listeler ve bir uyarıdan sonra kural ekler; web arayüzünde yalnızca bir `admin` için |
| Korunan bir ref'i herhangi bir programla silmek ya da geri almak | `conflict` ve kalan gün sayısıyla reddedilir |

## 6. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Bir ref'i kaldırılana kadar kuralının ötesinde tutan yasal saklama (legal hold) | Sonra, bir pilot gerektirirse; kaldırılabilen bir saklama tam da uyumluluk kurallarının yasakladığı şeydir, bu yüzden kendi tasarımını ister |
| SFTP ya da klasör uzak depoları için Object Lock | Apeiron'da mümkün değil; böyle bir depolamanın ZFS ya da bir yedek cihazınınki gibi kendi değiştirilemez anlık görüntüleri gerekir |
| Korunan bir geçmişin ortasından tek tek eski snapshot'ları silmek | Planlanmadı: ortası kaybolabilen bir geçmiş arşiv değildir |

## 7. Crate'ler

| Crate | Görevi | Durum |
| --- | --- | --- |
| `apeiron-core` | Kural dosyaları, her ref değişikliğinin denetimleri, kuralları push ve pull ile kopyalamak | Tamamlandı |
| `apeiron` | `retention`, `add_retention` | Tamamlandı |
| `apeiron-remote` | Her S3 yazmasında SHA-256 sağlama toplamları | Tamamlandı |
| `apeiron-cli` | `apeiron retention` | Tamamlandı |
| `apeiron-server` | Saklama sayfası | Tamamlandı |
