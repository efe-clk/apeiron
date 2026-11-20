> **English:** [USERS.md](USERS.md)

# Kullanıcılar ve İzinler

> **Durum: Tamamlandı.** Aşama 4'ün 3. işinin tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §6): birkaç kişi web arayüzünde ([WEB.tr.md](WEB.tr.md)) oturum açar; her birinin bir adı, parolası, rolü, görebileceği yedekleri ve kullanabileceği sunucu klasörleri vardır. Arkasındaki karar ADR-028'dir ([DECISIONS.tr.md](DECISIONS.tr.md)).

Bir aile ya da küçük bir ofis tek bir yedek sunucusunu paylaşır: herkes kendi klasörlerini yedekler ve geri yükler, yalnızca kendi yedeklerini görür; içlerinden biri bütün sunucuyla ilgilenir.

## 1. Roller

| Hak | `admin` | `member` | `reader` |
| --- | --- | --- | --- |
| Yedekleri ve sürümlerini görmek, içine bakmak, karşılaştırmak, aramak | Hepsi | Kendi yedekleri | Kendi yedekleri |
| Bir klasörü yedeklemek | Herhangi bir yedeğe | Kendi yedeklerine | Hayır |
| Geri yüklemek | Evet | Evet | Hayır |
| Depoyu denetlemek | Evet | Hayır | Hayır |
| Saklama kurallarını görmek ve eklemek ([ARCHIVE.tr.md](ARCHIVE.tr.md)) | Evet | Hayır | Hayır |
| Yasal saklama kilitlerini görmek ve koymak, uyumluluk raporu oluşturmak ([ARCHIVE.tr.md](ARCHIVE.tr.md) §2.1, [REPORTS.tr.md](REPORTS.tr.md)) | Evet | Hayır | Hayır |
| Sayfalarda uzak depolar, senkron, parolalar ve eklentiler ([WEB.tr.md](WEB.tr.md) §4) | Evet | Hayır | Hayır |
| `git push` ([GIT.tr.md](GIT.tr.md) §7.1) | Her dala | Kendi yedeklerine | Hayır |
| Bağlantıyla paylaşmak ([SHARE.tr.md](SHARE.tr.md)) | Her sürümü | Gördüğü sürümleri | Hayır |
| Sunucunun klasörleri | Kendi klasörleri, hiç yoksa bütün `--folder` klasörleri | Yalnızca kendi klasörleri | Hiçbiri |

Bir kullanıcının **yedekleri** ref adlarından oluşan bir listedir; `*` ile biten bir ad, kendisinden önceki kısımla başlayan her ref'in yerini tutar; yani `backup/ayse-*`, `backup/ayse-photos` ve `backup/ayse-documents` yedeklerini kapsar. Bir kullanıcının **klasörleri** sunucunun klasörleridir; yalnızca `apeiron web` komutunun bir `--folder` klasörünün içindekiler sayılır.

Bir sürüm, kullanıcının yedeklerinden birinin en yeni sürümünden üst sürümler yoluyla ulaşılabiliyorsa o kullanıcıya görünür. Bu yüzden başka bir yedeği birleştirerek oluşan bir yedek, aldığı sürümleri de gösterir.

## 2. Kullanıcılar dosyası

Kullanıcılar, yapılandırma klasöründeki ve yalnızca sahibinin okuyabildiği `web.json` dosyasında tutulur:

```json
{
  "users": [
    {"name": "efe", "password": "$argon2id$v=19$...", "role": "admin"},
    {"name": "ayse", "password": "$argon2id$v=19$...", "role": "member",
     "backups": ["backup/ayse-*"], "folders": ["/home/ayse"]}
  ]
}
```

| Alan | Anlamı |
| --- | --- |
| `name` | 1 ile 32 arasında küçük harf, rakam, `-` ve `_` |
| `password` | En az 8 karakterlik parolanın Argon2id özeti; `oidc` alanı olan bir kullanıcı için boş bırakılabilir |
| `oidc` | Kullanıcının, kurumun kimlik sağlayıcısındaki kimliği ([IDENTITY.tr.md](IDENTITY.tr.md)) |
| `from_groups` | Sağlayıcının gruplarından oluşturulmuş, rolü, yedekleri ve klasörleri onlara uyan bir kullanıcı için `true` ([IDENTITY.tr.md](IDENTITY.tr.md) §2.1) |
| `role` | `admin`, `member` ya da `reader` |
| `backups` | Ref adları, önek için sonda `*` ile; `admin` için kullanılmaz |
| `folders` | Sunucunun klasörleri |
| `max_bytes` | Yedeklerinin birlikte en çok ne kadar tutabileceği (§2.1); boş bırakılırsa sınır yok |
| `disabled` | SCIM ile kapatılmış bir kullanıcı için `true` (§4.1): oturum açamaz, oturumları ve erişim anahtarları bir sonraki istekte durur |

Sunucu dosya değişince onu yeniden okur; böylece çalışırken eklenen, değiştirilen ya da silinen bir kullanıcı bir sonraki istekten itibaren geçerli olur. Kullanıcısı silinen ya da parolası değişen bir oturum biter.

### 2.1 Kotalar

Bir kullanıcının `max_bytes`'ı, sayfalarda ya da JSON API'de başlattığı her yedek ve senkrondan önce denetlenir: kullanabileceği her ref'in (`backups`, bir `admin` için `apeiron/` altındakiler dışındaki her ref) en yeni snapshot'ları dosyalarının boyutuyla toplanır; sınırı ya da daha fazlasını tutuyorlarsa işlem "your backups hold … bytes, your limit is …" ile reddedilir. Eski sürümler ve birkaç yedeğin paylaştığı dosyalar, onları tutan her en yeni snapshot'ta sayılır; böylece sayı, kullanıcının hepsini geri yükleyerek alacağıdır. Diskteki yer kiracının sınırıdır (TENANTS.tr.md §2.1). Geri yükleme, okuma ve silme sürer.

## 3. Komutlar

| Komut | Ne yapar |
| --- | --- |
| `apeiron web user add <name> --role <role> [--backup <ref>]... [--folder <path>]... [--max-bytes <size>]` | Bir kullanıcı ekler; parolayı iki kez sorar |
| `apeiron web user change <name> [--role <role>] [--backup <ref>]... [--folder <path>]... [--max-bytes <size>]` | Verilen rolü, yedekleri, kotayı ya da klasörleri değiştirir |
| `apeiron web user password <name>` | Yeni bir parola belirler; kullanıcının oturumlarını bitirir |
| `apeiron web user second-factor <name> [--off]` | Parolayla oturum açmanın bir doğrulama uygulamasının kodunu da istemesini sağlar ve uygulamaya girilecek gizli anahtarı gösterir; `--off` kapatır; kullanıcının oturumlarını bitirir ([IDENTITY.tr.md](IDENTITY.tr.md) §4) |
| `apeiron web user remove <name>` | Bir kullanıcıyı siler; son `admin` silinemez |
| `apeiron web user list` | Kullanıcıları rolleri, yedekleri ve klasörleriyle listeler |

`apeiron web` en az bir `admin` olmadan başlamaz.

## 4. Sayfalarda ve JSON API'de

| İstek | Değişiklik |
| --- | --- |
| `POST /api/login` `{"name", "password"}` | Kullanıcı olarak oturum açar; yanlış ad ve yanlış parola aynı işi yaptıktan sonra aynı hatayı verir |
| `GET /api/session` | Oturum açıksa `"user": {"name", "role"}` ekler |
| `POST /api/call/refs` | Yalnızca kullanıcının yedeklerinin ref'leri |
| Bir snapshot adlandıran istekler (`spec`, `start`, `snapshot`, `from`, `to`) | Snapshot kullanıcıya görünür olmalıdır (§1); kimliğiyle değiştirilir, böylece denetlenen neyse kullanılan da odur |
| `POST /api/call/reflog` | Yalnızca kullanıcının yedeklerinin ref'leri için |
| İncelemeler (`reviews`, `review`, `open_review`, `note_review`, `merge_review`) | Yalnızca iki ref'i de kullanıcının yedekleri olan incelemeler; `author` ne derse desin her adım oturum açan kullanıcının adıyla yazılır ([REVIEW.tr.md](REVIEW.tr.md)) |
| `POST /api/start/search` | Bir `member` ya da `reader` bir snapshot belirtmelidir |
| `GET /api/folders` | Yalnızca kullanıcının klasörlerinin içinde |
| Rolün ötesindeki her şey | `unsupported` ile reddedilir; başka bir ref'e yedek ya da klasörlerin dışındaki bir yol `invalid_input` ile |
| `GET /api/users` | Bir `admin` için: parolanın özeti olmadan her kullanıcı, parolası olup olmadığı ve kullanıcılara verilebilecek klasörler |
| `POST /api/users` `{"name", "role", "password", "oidc", "backups", "folders"}` | Bir `admin` için: §3'teki denetimlerle bir kullanıcı ekler; klasörler bir `--folder` içinde olmalıdır |
| `POST /api/users/<ad>` | Bir `admin` için: gönderilen alanları değiştirir; boş bir `password` onu korur, `"no_password": true` onu kaldırır. Bir admin kendi admin rolünü kaldıramaz ve her zaman bir admin vardır |
| `POST /api/users/<ad>/remove` | Bir `admin` için: kendisi dışındaki bir kullanıcıyı kaldırır |

Sayfalar kullanıcının adını Oturumu kapat'ın yanında gösterir ve rolün yapamadıklarını gizler: bir `reader` için Yedekle ve Geri yükle, `admin` dışındaki herkes için Denetle. Bir `admin`'in ayrıca, yukarıdaki isteklerle kullanıcı ekleyen, değiştiren ve kaldıran bir Kullanıcılar sayfası vardır; her değişiklik o admin'in adıyla denetim kaydına yazılır ve değişen bir parola ya da kimlik kullanıcının oturumlarını bitirir. Sayfaların ve `apeiron web user` komutunun değişiklikleri birer birer ve bütün olarak yazılır (`web.json.lock`).

### 4.1 SCIM

Microsoft Entra ID ya da Okta gibi bir kurumun kimlik sağlayıcısı, bir web sunucusunun kullanıcılarını `https://<server>/scim/v2` adresinde SCIM 2.0 ile kendi kullanıcılarıyla eşit tutar; gizli anahtar olarak bir `admin`'in erişim anahtarını kullanır ([API.tr.md](API.tr.md) §2.1). Her değişiklik denetim kaydına `scim_user_add`, `scim_user_change` ya da `scim_user_remove` olarak yazılır.

| İstek | Ne yapar |
| --- | --- |
| `GET /scim/v2/ServiceProviderConfig` | Neyin desteklendiği: aşağıdaki istekler, `PATCH` ve `userName eq "…"` süzgeci |
| `GET /scim/v2/Users[?filter=userName eq "…"][&startIndex=…&count=…]` | Kullanıcıları bir seferde en çok 200 olarak listeler |
| `POST /scim/v2/Users` `{"userName", "active"}` | Sağlayıcı üzerinden oturum açan bir kullanıcı oluşturur: `userName` onun kimliği (`oidc`), adı da gruplardaki gibi `@`'den önceki kısım olur ([IDENTITY.tr.md](IDENTITY.tr.md) §2.1); `backup/<name>-*` yedekleriyle ve klasörsüz bir `member`'dır, bir admin bunları değiştirebilir. Bir kullanıcı o kimliğe zaten sahipse `409` |
| `GET`, `PUT`, `PATCH /scim/v2/Users/<id>` | `userName` ve `active`'i gösterir ya da değiştirir; `active: false` kullanıcıyı kapatır (`disabled`). Adlar ve e-posta adresleri gibi diğer özellikler sağlayıcıda kalır |
| `DELETE /scim/v2/Users/<id>` | Kullanıcıyı siler; son `admin` silinemez |

`id` kullanıcının adıdır. Gruplar aktarılmaz: roller, yedekler ve klasörler yukarıdaki varsayılanlardan, bir adminden ya da oturum açarken grup kurallarından gelir.

## 5. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Kimin ne yaptığının kaydı | 4. iş ([AUDIT.tr.md](AUDIT.tr.md)) |
| Kullanıcı başına ayrı şifreleme anahtarları | Kullanıcılar deponun anahtarını paylaşır; anahtarı sunucu tutar |
| Masaüstü arayüzünde kullanıcılar | Gerekmez: yalnızca onu çalıştıran kullanıcıya hizmet eder |
| Merkezi kimlik | Aşama 5'in 1. işi ([IDENTITY.tr.md](IDENTITY.tr.md)) |
| Bir sunucuda birbirinden ayrı tutulan birkaç kurum | Aşama 5'in 2. işi ([TENANTS.tr.md](TENANTS.tr.md)) |

## 6. Crate'ler

| Crate | Görevi | Durum |
| --- | --- | --- |
| `apeiron-server` | Kullanıcılar, kullanıcı başına oturumlar ve §4'teki denetimler | Tamamlandı |
| `apeiron-cli` | `apeiron web user` | Tamamlandı |
