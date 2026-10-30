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
| `password` | En az 8 karakterlik parolanın Argon2id özeti |
| `role` | `admin`, `member` ya da `reader` |
| `backups` | Ref adları, önek için sonda `*` ile; `admin` için kullanılmaz |
| `folders` | Sunucunun klasörleri |

Sunucu dosya değişince onu yeniden okur; böylece çalışırken eklenen, değiştirilen ya da silinen bir kullanıcı bir sonraki istekten itibaren geçerli olur. Kullanıcısı silinen ya da parolası değişen bir oturum biter.

## 3. Komutlar

| Komut | Ne yapar |
| --- | --- |
| `apeiron web user add <name> --role <role> [--backup <ref>]... [--folder <path>]...` | Bir kullanıcı ekler; parolayı iki kez sorar |
| `apeiron web user change <name> [--role <role>] [--backup <ref>]... [--folder <path>]...` | Verilen rolü, yedekleri ya da klasörleri değiştirir |
| `apeiron web user password <name>` | Yeni bir parola belirler; kullanıcının oturumlarını bitirir |
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
| `POST /api/start/search` | Bir `member` ya da `reader` bir snapshot belirtmelidir |
| `GET /api/folders` | Yalnızca kullanıcının klasörlerinin içinde |
| Rolün ötesindeki her şey | `unsupported` ile reddedilir; başka bir ref'e yedek ya da klasörlerin dışındaki bir yol `invalid_input` ile |

Sayfalar kullanıcının adını Oturumu kapat'ın yanında gösterir ve rolün yapamadıklarını gizler: bir `reader` için Yedekle ve Geri yükle, `admin` dışındaki herkes için Denetle.

## 5. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Kullanıcıları sayfalarda yönetmek | Sonra; o zamana kadar §3'teki komutlar |
| Kimin ne yaptığının kaydı | 4. iş ([AUDIT.tr.md](AUDIT.tr.md)) |
| Kullanıcı başına ayrı şifreleme anahtarları | Kullanıcılar deponun anahtarını paylaşır; anahtarı sunucu tutar |
| Masaüstü arayüzünde kullanıcılar | Gerekmez: yalnızca onu çalıştıran kullanıcıya hizmet eder |
| Merkezi kimlik (LDAP, OIDC) | Aşama 5 |

## 6. Crate'ler

| Crate | Görevi | Durum |
| --- | --- | --- |
| `apeiron-server` | Kullanıcılar, kullanıcı başına oturumlar ve §4'teki denetimler | Tamamlandı |
| `apeiron-cli` | `apeiron web user` | Tamamlandı |
