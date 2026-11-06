> **English:** [GIT.md](GIT.md)

# Apeiron ve Git

> **Durum: Tamamlandı.** Aşama 3'ün 1. işinin tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §5): bir Git deposunu geçmişiyle birlikte içe almak ve geri dışa aktarmak. Arkasındaki karar ADR-021'dir ([DECISIONS.tr.md](DECISIONS.tr.md)).

Aşama 3, gerçek bir Git deposu Apeiron'a alınıp geri verilebildiğinde biter. "Geri" katı ölçülür: yeniden dışa aktarılan her commit ve açıklamalı etiket, sahip olduğu Git nesne kimliğinin aynısını alır; böylece Git'teki geçmiş değişmez, imzalar geçerli kalır ve `git fsck` hiçbir şey bulmaz.

## 1. Komutlar

| Komut | Ne yapar | Durum |
| --- | --- | --- |
| `apeiron git import <git-repo> [--prefix <name>]` | Bir Git deposunun her dalını, etiketini ve uzak takip dalını tüm geçmişiyle snapshot olarak saklar; ref'ler `<prefix>/heads/…`, `<prefix>/tags/…` ve `<prefix>/remotes/…` olur (varsayılan önek `git`). Yeniden çalıştırılınca yalnızca yeni commit'leri saklar | Tamamlandı |
| `apeiron git export <git-repo> [--prefix <name>]` | Bu ref'lerin snapshot'larını Git nesneleri olarak bir Git deposuna yazar (yoksa çıplak bir depo oluşturur) ve dallarını, etiketlerini ve uzak takip dallarını onlara yöneltir; bir commit başka bir kimlik aldıysa çıkış kodu 3'tür | Tamamlandı |

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

Yalnızca `refs/heads/`, `refs/tags/` ve `refs/remotes/` içe alınır; bir klonda dalların çoğu uzak takip dallarıdır. `HEAD`, `origin/HEAD` gibi sembolik ref'ler, notlar ve stash'ler dışarıda kalır ve listelenir. Git'te silinen bir dal Apeiron ref'ini korur; içe alma onu eskimiş olarak raporlar.

## 4. Çalıştırmalar: içe alma

1. Her dalı, etiketi ve uzak takip dalını oku. Bir tree'ye ya da blob'a işaret eden bir etiket ya da etiket zinciri bir uyarıyla atlanır.
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

## 7. HTTP üzerinden Git

`apeiron web` (WEB.tr.md), bir ref önekinin altındaki dalları ve etiketleri Git'e HTTP üzerinden sunar; böylece Apeiron'da tutulan bir proje dışa ya da içe aktarılmadan klonlanabilir, çekilebilir ve ona push edilebilir:

```bash
git clone https://ayse@backup.example.com:8443/git/git    # git/ altındaki ref'ler, içe aktarmanın varsayılan öneki
```

Adres, sonunda `.git` olsun ya da olmasın, `/git/<prefix>`'tir. Çekme aşağıdaki gibi işler; push bölüm 7.1'dedir.

1. Git her istekte kullanıcının adı ve parolasıyla (HTTP Basic) oturum açar; yanlış parolalar sayfalardaki oturum açmalar gibi bekletilir (WEB.tr.md §5). İkinci doğrulaması olan (IDENTITY.tr.md §4) ya da parolası olmayan bir kullanıcı bunu kullanamaz, çünkü Git kodu gönderemez.
2. Sunucu önekin altındaki ref'leri, denetim kaydının klasöründeki `git/<prefix>.git` adlı kendi Git kopyasına aktarır ve her istekte, `apeiron git export`'un yaptığı gibi (bölüm 5) günceller. Kopya her an silinebilir; yeniden oluşturulur.
3. Önekin altında kullanıcının gördüğü (USERS.tr.md §1) dalları ve etiketleri sunar; `HEAD`, `main`'i, yoksa `master`'ı, yoksa ilk dalı gösterir. Yalnızca bunlar istenebilir; böylece kullanıcıya yedeklerinin ulaşmadığı hiçbir şey gitmez.
4. Commit'leri zaten olan bir istemci onları adlandırır (`have`). İstenen ref'lerin ulaştığı ilki ortaktır: sunucu bunu, Git'in `multi_ack` olmadan yaptığı gibi söyler (`ACK`) ve paket ortak commit'lerin ulaştığı her şeyi dışarıda bırakır; böylece bir fetch yalnızca yeni olanı indirir. Diğer adlar geçilir; böylece yanıt, kullanıcıya sunulmayan ref'ler hakkında hiçbir şey söylemez.
5. Yanıt bu nesnelerin bir paketidir. Git'in yaptığı gibi aynı adlı ağaçlar ve dosyalar büyükten küçüğe sıralanır ve her biri kendinden önceki on nesneye karşı denenir: farklar boyutunun yarısından az yer tutuyorsa delta olarak, yani o nesneden kopyalar ve yeni baytlar olarak, en çok 50 delta derinliğine kadar gönderilir. 16 MiB'tan büyük nesneler bütün gönderilir. Bir paket, daha çok arayan Git'in kendi paketinden yine biraz büyüktür.
6. Her fetch önek ve ref'lerle `git_fetch` olarak (AUDIT.tr.md §1), yanlış bir parola da `"via": "git"` ile `sign_in` olarak kaydedilir.

Sunucu Git'in akıllı HTTP protokolünün 0. sürümünü side-band olmadan konuşur; yeni Git sürümleri 2. sürümü ister ve buna geri döner.

### 7.1 Push

`git push`, yeni commit'leri `apeiron git import`'un yapacağı gibi snapshot olarak saklar ve ref'leri taşır. Yalnızca yedek alabilen bir kullanıcı (USERS.tr.md §1: `admin` ya da kendi yedekleri için bir `member`) push edebilir; bir yedek sunucuya (REPLICATION.tr.md §5) edemez.

1. Sunucu kullanıcının gördüğü ref'leri sunar; dışarıdaki nesnelere dayanmayan bir paket (`no-thin`) ve ref başına bir rapor (`report-status`) ister; silmeleri (`delete-refs`) de kabul eder.
2. Git kopyasını günceller, sonra paketin her nesnesini deltaları tabanlarına göre bütünleyerek ona yazar. Bir push en çok 256 MiB olabilir; daha büyük bir geçmiş parça parça push edilir.
3. Her ref denetlenir: önekin altında, kullanıcının sahip olduğu bir dal ya da etiket olmalı ve eski kimliği kopyadakiyle aynı olmalıdır (yoksa "önce fetch et"). Bir ref silinebilir (`git push origin :<branch>`) ya da geçmişinde eskisi bulunmayan bir commit'e taşınabilir (`git push --force`); gösterdiği sürümler reflog'da kalır ve `apeiron gc` onları `--keep-days` süresince tutar. Bir saklama kuralının tuttuğu ref ne silinir ne geri alınır (ARCHIVE.tr.md).
4. Kabul edilen ref'lerin ulaştığı ve henüz saklanmamış commit'ler bölüm 2.2'deki Git alanlarıyla snapshot olur; böylece dışa aktarıldıklarında aynı kimlikleri alırlar. Sonra her ref karşılaştır-ve-değiştir ile taşınır; bu arada başka bir programın taşıdığı ref reddedilir.
5. Git her ref için `ok` ya da nedenini gösterir. Push, her ref, yeni kimliği ve hatasıyla `git_push` olarak kaydedilir (AUDIT.tr.md §1) ve bir ref'i taşıyan push, bir yedek gibi yedek sunuculara kopyalanır (REPLICATION.tr.md §4).

## 8. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| SSH üzerinden Git | Daha sonra; HTTP (bölüm 7) bir projeyi Apeiron'a alır ve oradan çıkarır |
| Bir Git çalışma ağacına checkout | `apeiron restore` bir snapshot'ı bir klasöre yazar |
| Apeiron içinde dallanma ve birleştirme | [BRANCH.tr.md](BRANCH.tr.md) |
| Kodu eklenti arayüzünün arkasına taşımak | Yapılmadı; içe aktarıcı eklenti türü gerekir ([ROADMAP.tr.md](ROADMAP.tr.md) §8) |

## 9. Crate'ler ve kütüphaneler

| Crate | Görev | Durum |
| --- | --- | --- |
| `apeiron-git` | İçe ve dışa aktarma, sunulan ref'ler ve paketler, alınan paketler (bölüm 7); `gix`'i yalnızca o kullanır | Tamamlandı |
| `apeiron` | `git_import` ve `git_export` işleri (API.tr.md §8.3); sunucu için `git_serve_refs`, `git_common`, `git_pack` ve `git_receive` | Tamamlandı |
| `apeiron-server` | `/git/<prefix>/info/refs`, `/git/<prefix>/git-upload-pack` ve `/git/<prefix>/git-receive-pack` (bölüm 7) | Tamamlandı |
| `apeiron-cli` | `apeiron git import` ve `apeiron git export` | Tamamlandı |

Nesneleri ve ref'leri `gix` (gitoxide) 0.85 okur ve yazar (VISION.tr.md). Nesneler ham bayt olarak çözülür ve yazılır; böylece gix'in normalleştirdiği hiçbir şey bir kimliği değiştiremez.

Testler `git` ile; bir birleştirme, büyük harfli bir dal, çalıştırılabilir bir dosya, bir sembolik bağ, bir alt modül girdisi, iki tür etiket, bir imza başlığı ve Latin-1 bir mesaj içeren bir depo kurar ve her kimliğin geri geldiğini denetler. Gerçek depolar da aynı şekilde geri geldi: 3.568 commit'lik bir depo; 4.265 commit ve 13 etiketli bir depo; ve 22 saniyede içe alınıp 19 saniyede dışa aktarılan 27.156 commit ve 70 etiketli bir depo.
