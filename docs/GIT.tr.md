> **English:** [GIT.md](GIT.md)

# Apeiron ve Git

> **Durum: Devam ediyor.** Aşama 3'ün 1. işinin tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §5): bir Git deposunu geçmişiyle birlikte içe almak ve geri dışa aktarmak. Arkasındaki karar ADR-021'dir ([DECISIONS.tr.md](DECISIONS.tr.md)).

Aşama 3, gerçek bir Git deposu Apeiron'a alınıp geri verilebildiğinde biter. "Geri" katı ölçülür: yeniden dışa aktarılan her commit ve açıklamalı etiket, sahip olduğu Git nesne kimliğinin aynısını alır; böylece Git'teki geçmiş değişmez, imzalar geçerli kalır ve `git fsck` hiçbir şey bulmaz.

## 1. Komutlar

| Komut | Ne yapar | Durum |
| --- | --- | --- |
| `apeiron git import <git-repo> [--prefix <name>]` | Bir Git deposunun her dalını ve etiketini tüm geçmişiyle snapshot olarak saklar; ref'ler `<prefix>/heads/…` ve `<prefix>/tags/…` olur (varsayılan önek `git`). Yeniden çalıştırılınca yalnızca yeni commit'leri saklar | Planlandı |
| `apeiron git export <git-repo> [--prefix <name>]` | Bu ref'lerin snapshot'larını Git nesneleri olarak bir Git deposuna yazar (yoksa çıplak bir depo oluşturur) ve dallarını ve etiketlerini onlara yöneltir | Planlandı |

Git'ten yapılan snapshot'lar sıradan snapshot'lardır: `apeiron log git/heads/main`, `apeiron diff`, `apeiron restore git/heads/main ~/checkout` ve bir uzak depoya push onlarla çalışır. Apeiron'da yapılan bir snapshot, örneğin bir yedek, ref'i önekin altındaysa yeni bir commit olur.

## 2. Git nesnelerinden snapshot'lara

| Git | Apeiron | Onu kesin tutan |
| --- | --- | --- |
| Blob | Her dosya gibi chunk'lara bölünmüş blob | İçerik |
| Tree | Zamanlar ve sahipler olmadan tree | Girdi adları ve türleri; `mode` içindeki dosya kipi (bölüm 2.1) |
| Commit | Commit'in tree'sini ve ebeveynlerinin snapshot'larını taşıyan snapshot | `meta` içindeki ham başlıklar ve mesaj (bölüm 2.2) |
| Açıklamalı etiket | Tek ebeveyni etiketlenen commit'in snapshot'ı olan, o commit'in tree'sini taşıyan snapshot | `meta` içindeki ham başlıklar ve mesaj |
| Dal, hafif etiket | Ref | Kaçışlı ad (bölüm 3) |
| Alt modül girdisi (`160000`) | Tree'de değil; snapshot'ın `meta`'sında listelenir | Yol ve commit kimliği |

### 2.1 Tree girdileri

| Git kipi | Apeiron girdisi |
| --- | --- |
| `100644`, `100755` ve eski `100664` | Dosya; `mode` son üç sekizlik basamaktır |
| `120000` | Sembolik bağ; blob içeriği hedeftir |
| `40000` | Klasör |
| `160000` | Tree'ye girmez ve `git.gitlinks` içinde listelenir (bölüm 2.2); yalnızca alt modül tutan bir klasör boş klasör olarak kalır |

Dışa aktarma bir dosya kipini `100` ve ardından sekizlik `mode` olarak geri yazar; böylece eski `100664` korunur. Git bir klasörü, adı `/` ile bitiyormuş gibi sıralar; dışa aktarma, Apeiron tree'sinin sırası ne olursa olsun, böyle sıralar.

### 2.2 Snapshot alanları ve `meta` anahtarları

| Alan ya da anahtar | İçerik |
| --- | --- |
| `time` | Commit'i yazanın zamanı (etiket için etiketleyenin zamanı) |
| `author` | Yazarın (etiket için etiketleyenin) `Ad <e-posta>` bilgisi |
| `message` | Geçerli UTF-8 ise mesaj |
| `git.id` | Git nesne kimliği, 40 onaltılık basamak; dışa aktarma onun yeniden üretildiğini denetler |
| `git.kind` | `commit` ya da `tag` |
| `git.headers` | `tree` ve `parent` satırlarından (etiket için `object` satırından) sonraki her başlık satırı, saklandığı gibi: yazar, commit'i yazan, kodlama, imzalar ve diğerleri. Geçerli UTF-8 değillerse anahtar `git.headers-hex` olur ve baytlar onaltılık yazılır |
| `git.message-hex` | Geçerli UTF-8 değilse mesaj baytları, onaltılık |
| `git.no-message` | Nesnede boş satır ve hiç mesaj yoksa `1` |
| `git.gitlinks` | Her alt modül girdisi için bir satır: onaltılık yol, bir boşluk, commit kimliği |

`meta`, hizmetlerin serbestçe kullanabileceği bir alandır (FORMAT.tr.md §6.4); bu anahtarlar formatta bir değişiklik gerektirmez.

## 3. Ref adları

Git, `feature/ABC-123` içindeki büyük harfler gibi, Apeiron ref adlarının izin vermediği adlara izin verir (FORMAT.tr.md §9.3). Bir Git adının her parçası kaçışlanır: `a-z`, `0-9`, `.` ve `-` kalır; bir parçanın başındaki `.` ve diğer her bayt `_` ve iki küçük onaltılık basamak olur. `feature/ABC-123`, `git/heads/feature/_41_42_43-123` olur; dışa aktarma bunu geri çevirir. Kaçıştan sonra 200 bayttan uzun bir ad bir uyarıyla atlanır.

Yalnızca `refs/heads/` ve `refs/tags/` içe alınır. `HEAD`, uzak takip ref'leri, notlar ve stash'ler dışarıda kalır ve listelenir. Git'te silinen bir dal Apeiron ref'ini korur; içe alma onu eskimiş olarak raporlar.

## 4. Çalıştırmalar: içe alma

1. Her dalı ve etiketi oku. Bir tree'ye ya da blob'a işaret eden bir etiket ya da etiket zinciri bir uyarıyla atlanır.
2. Zaten saklananı bul: önekin altındaki mevcut ref'lerin snapshot'larını dolaş ve `git.id` değerlerini oku.
3. Eksik commit'leri, ebeveynler çocuklardan önce olmak üzere sakla. Bir çalıştırma içinde daha önce görülen blob'lar ve tree'ler yeniden okunmaz.
4. Her ref'i karşılaştır ve değiştir ile snapshot'ına taşı. Git'te geriye giden (zorla push edilen) bir dal ref'i de taşır; reflog eski snapshot'ı tutar.

Önce nesneler, en son ref'ler yazılır (FORMAT.tr.md §9.2); böylece yarıda kalan bir içe alma, eksik bir şeye işaret eden hiçbir ref bırakmaz ve sonraki çalıştırma zaten sakladığı commit'leri bulur.

## 5. Çalıştırmalar: dışa aktarma

1. Git deposunu aç ya da çıplak bir depo oluştur; yeni bir deponun `HEAD`'i `main`'e, yoksa `master`'a, o da yoksa ilk dala işaret eder.
2. Önekin altındaki her ref için snapshot'larını, ebeveynler önce olmak üzere commit ya da etiket olarak yaz. `git.id`'si Git deposunda zaten olan bir snapshot yeniden yazılmaz.
3. Her nesneyi alanlarından yeniden kur; `git.id`'si olan bir commit o kimliği almalıdır. Almayan bir commit yine yazılır ve kesin olmayan olarak raporlanır.
4. Her dalı ve etiketi nesnesine yönelt.

Git alanları olmayan bir snapshot; snapshot'ın yazarıyla (ya da `Apeiron <apeiron@localhost>`), yazar ve commit zamanı olarak UTC'deki zamanıyla ve mesajıyla bir commit olur. Dışa aktarma gevşek nesneler yazar; `git gc` onları paketler.

## 6. Sınırlar

| Durum | Ne olur |
| --- | --- |
| SHA-256 depoları | Reddedilir; Apeiron SHA-1 depolarını okur ve yazar |
| Sığ klonlar | Reddedilir: eksik ebeveynler saklanamaz |
| Eski bozuk araçların yazdığı gibi girdileri yanlış sırada olan tree'ler | İçe alınır; dışa aktarma kesin olmayan olarak raporlanır |
| Alt modül içeriği | İçe alınmaz; alt modülün kendi deposu ayrıca içe alınabilir |
| Git LFS | Büyük dosyalar değil, işaretçi dosyalar içe alınır |

## 7. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Git ağ protokolü (SSH ya da HTTP üzerinden clone ve push) | Aşama 3'te daha sonra; şart değil, hedef (VISION.tr.md) |
| Bir Git çalışma ağacına checkout | `apeiron restore` bir snapshot'ı bir klasöre yazar |
| Apeiron içinde dallanma ve birleştirme | Aşama 3'ün 2. işi |
| Kodu eklenti arayüzünün arkasına taşımak | Eklenti sistemiyle, Aşama 3 |

## 8. Crate'ler ve kütüphaneler

| Crate | Görev | Durum |
| --- | --- | --- |
| `apeiron-git` | İçe ve dışa aktarma; `gix`'i yalnızca o kullanır | Planlandı |
| `apeiron` | `git_import` ve `git_export` işleri (API.tr.md §8.3) | Planlandı |
| `apeiron-cli` | `apeiron git import` ve `apeiron git export` | Planlandı |

Nesneleri ve ref'leri `gix` (gitoxide) 0.85 okur ve yazar (VISION.tr.md); Rust 1.85 ile derlenen en yeni sürüm odur. Nesneler ham bayt olarak çözülür ve yazılır; böylece gix'in normalleştirdiği hiçbir şey bir kimliği değiştiremez.
