> **English:** [IDENTITY.md](IDENTITY.md)

# Merkezi Kimlik

> **Durum: Tamamlandı.** Aşama 5'in 1. işinin tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §7): web arayüzünün ([WEB.tr.md](WEB.tr.md)) kullanıcıları, Apeiron'un tuttuğu bir parola yerine, OpenID Connect konuşan bir kimlik sağlayıcısı üzerinden kurumlarının hesabıyla oturum açar. Arkasındaki karar ADR-031'dir ([DECISIONS.tr.md](DECISIONS.tr.md)).

Bir kurumun, çalışanlarının oturum açtığı tek bir yeri zaten vardır: Keycloak, Microsoft Entra ID, Google Workspace, Okta ya da Authentik gibi. Bu yer parolaları ve ikinci adımları denetler, ayrılan kişilerin hesaplarını kapatır. Yedek sunucusu kendi parolalarını tutmak yerine onu kullanmalıdır.

## 1. Neler değişir

| Önce | Bir kimlik sağlayıcısıyla |
| --- | --- |
| Her kullanıcının `web.json` içinde bir parolası vardır | Bir kullanıcının bunun yerine sağlayıcıda bir kimliği olabilir, ya da ikisi birden |
| Parolayı Apeiron denetler | Kişiyi sağlayıcı, kendi ikinci adımları ve kurallarıyla denetler |
| Erişimi kaldırmak kullanıcıyı silmek demektir | Hesabı sağlayıcıda kapatmak yeni oturum açmaları da durdurur |

Roller, yedekler ve klasörler `web.json` içinde kalır (USERS.tr.md §1): sağlayıcı birinin kim olduğunu söyler, Apeiron ne yapabileceğini. İlk `admin` gibi parolası olan bir kullanıcı onunla oturum açmaya devam edebilir; böylece sağlayıcı çalışmadığında da sunucuya ulaşılabilir.

## 2. Ayarlar

Sağlayıcı, `web.json` içinde kullanıcıların yanında ayarlanır:

```json
{
  "oidc": {
    "issuer": "https://login.example.com/realms/corp",
    "client_id": "apeiron",
    "client_secret": "…",
    "address": "https://backup.example.com",
    "claim": "email"
  },
  "users": [
    {"name": "efe", "password": "$argon2id$v=19$...", "role": "admin"},
    {"name": "ayse", "role": "member", "oidc": "ayse@example.com",
     "backups": ["backup/ayse-*"], "folders": ["/home/ayse"]}
  ]
}
```

| Alan | Anlamı |
| --- | --- |
| `issuer` | Sağlayıcının yayıncı (issuer) adresi; ayarları `<issuer>/.well-known/openid-configuration` adresinden okunur |
| `client_id`, `client_secret` | Bu sunucu sağlayıcıya kaydedilirken sağlayıcının verdikleri; açık (public) bir istemci için gizli anahtar boş bırakılabilir |
| `address` | Tarayıcıların bu sunucuya ulaştığı adres; sağlayıcı onları `<address>/api/oidc/callback` adresine geri gönderir |
| `claim` | ID belirtecinin bir kullanıcıyı adlandıran alanı: varsayılan olarak `email`, ya da `sub`, `preferred_username` gibileri |
| Bir kullanıcının `oidc` alanı | O alanın kullanıcıdaki değeri; iki kullanıcının değeri aynı olamaz |
| Bir kullanıcının `password` alanı | Kullanıcının bir `oidc` kimliği varsa boş bırakılabilir |

Yayıncı ve adres `https` kullanmalıdır; denemek için bu bilgisayar (`localhost`, `127.0.0.1`) dışında. E-posta adresleri büyük-küçük harf ayrımı olmadan, diğer alanlar tam olarak karşılaştırılır.

### 2.1 Gruplardan kullanıcılar

Bir admin, her kişiyi `web.json` içinde listelemek yerine kimin hangi haklarla oturum açabileceğine sağlayıcının gruplarının karar vermesini sağlayabilir. Her kural bir grubu ve üyelerinin ne alacağını belirtir:

```json
"oidc": {
  "groups_claim": "groups",
  "groups": [
    {"group": "backup-admins", "role": "admin"},
    {"group": "staff", "role": "member", "backups": ["backup/{user}-*"],
     "folders": ["/srv/home/{user}"]}
  ]
}
```

| Alan | Anlamı |
| --- | --- |
| `groups_claim` | ID belirtecinin kullanıcının gruplarını listeleyen alanı, varsayılan olarak `groups`; bir ad listesi ya da tek bir ad. Sağlayıcı bunu belirtece koyacak biçimde ayarlanmalıdır |
| `group` | Sağlayıcının gönderdiği biçimiyle bir grup adı |
| `role`, `backups`, `folders` | Bir kullanıcıdaki gibi (USERS.tr.md §2); `{user}` kullanıcının adıyla değiştirilir |

Kimliği elle listelenmiş hiçbir kullanıcıya uymayan bir oturum açmada, belirtecin gruplarından birini adlandıran ilk kural kullanılır: kimlikten türetilen bir adla (`@` işaretinden önceki kısım; küçük harf, rakam, `-` ve `_` ile, alınmışsa `-2`, `-3` … eklenerek), parolasız ve `"from_groups": true` olan bir kullanıcı oluşturulur. Sonraki her oturum açmada rolü, yedekleri ve klasörleri uyan ilk kurala göre yeniden ayarlanır; böylece sağlayıcıdaki bir grup değişikliği bir sonraki oturum açmada geçerli olur. Artık adı geçen hiçbir grupta olmayan bir kullanıcı reddedilir; açık oturumları her zamanki sürelerinde biter. Elle listelenmiş bir kullanıcı (`from_groups` olmadan) gruplarla hiç değişmez ve bir admin, `from_groups` alanını kaldırarak oluşturulmuş bir kullanıcıyı böyle bir kullanıcıya çevirebilir.

## 3. Oturum açmak

```mermaid
sequenceDiagram
    participant B as Tarayıcı
    participant S as Apeiron sunucusu
    participant P as Kimlik sağlayıcısı
    B->>S: GET /api/oidc/start
    S->>S: yeni state, nonce ve PKCE doğrulayıcısı, 10 dakika tutulur
    S-->>B: sağlayıcıya 302, apeiron_oidc=state çerezi
    B->>P: oturum açar (parola, ikinci adım…)
    P-->>B: /api/oidc/callback?code&state adresine 302
    B->>S: GET /api/oidc/callback, çerezle
    S->>S: state çerezle aynı ve bir kez kullanılıyor
    S->>P: POST token adresi: code, PKCE doğrulayıcısı, gizli anahtar
    P-->>S: ID belirteci
    S->>S: imza, yayıncı, hedef, süre ve nonce denetimi
    S->>S: oidc alanı bu değer olan kullanıcıyı bulur
    S-->>B: / adresine 302, oturum çerezi
```

Sunucu bir ID belirtecini yalnızca bunların hepsi doğruysa kabul eder:

| Denetim | Ayrıntı |
| --- | --- |
| İmza | RS256 ya da ES256, sağlayıcının `jwks_uri` adresinde yayımladığı bir anahtarla; `none` ya da paylaşılan bir gizli anahtarla imzalanmış belirteçler reddedilir. Bir belirteç bilinmeyen bir anahtarı adlandırdığında anahtarlar, en çok dakikada bir, yeniden okunur |
| Yayıncı | `iss`, sağlayıcının yayıncısıdır |
| Hedef | `aud` istemci kimliğini içerir; başkalarını da içeriyorsa `azp` istemci kimliğidir |
| Zaman | `exp` geçmemiştir, bir dakika artı ya da eksi |
| Bu oturum açma | `nonce` bu oturum açma için gönderilendir ve state onu başlatan tarayıcıya dönmüştür |
| E-posta | `email` alanıyla, sağlayıcı onu doğrulanmamış olarak işaretlememiştir (`email_verified: false`) |

Ardından gelen oturum, bir paroladan sonrakiyle aynıdır (WEB.tr.md §5); kullanıcının kimliği, parolası ya da `web.json` içindeki kaydı değişince biter. Bu yüzden sağlayıcıda kapatılan bir hesabın zaten açık olan oturumu, oturum bitene ya da kullanıcı `web.json` dosyasından silinene kadar sürer. Başarısız bir oturum açma, tarayıcıyı bir iletiyle oturum açma ekranına döndürür; nedeni denetim kaydındadır.

## 4. Sayfalarda, JSON API'de ve CLI'da

| Nerede | Ne |
| --- | --- |
| Oturum açma ekranı | Bir sağlayıcı ayarlıysa "Kurumunuzla oturum açın" düğmesi |
| `GET /api/session` | Oturum kapalıyken ve bir sağlayıcı ayarlıyken `"oidc": true` |
| `GET /api/oidc/start` | Tarayıcıyı sağlayıcıya gönderir |
| `GET /api/oidc/callback` | Sağlayıcının tarayıcıyı geri gönderdiği yer; başarısız olursa `/#oidc_error=<tür>` adresine |
| `POST /api/logout` | Oturumu kapatır; sağlayıcı üzerinden açılmış bir oturumdan sonra yanıt `provider_sign_out` içerir: sağlayıcının `end_session_endpoint` adresi, `id_token_hint` olarak ID token, istemci kimliği ve `post_logout_redirect_uri` olarak `<address>/`. Sayfalar tarayıcıyı oraya gönderir; böylece kullanıcının sağlayıcıdaki oturumu da kapanır ve oturum açma ekranına döner. Bu adresi olmayan ya da ulaşılamayan bir sağlayıcıda oturum açık kalır |
| `apeiron web oidc set --issuer <url> --client-id <id> --address <url> [--claim <alan>] [--client-secret-file <yol>]` | Sağlayıcıyı ayarlar; gizli anahtarı sorar; sağlayıcıya kaydedilecek geri dönüş adresini ve oturum kapatıldıktan sonraki adresi (`<address>/`) yazar |
| `apeiron web oidc group <group> --role <role> [--backup <pattern>]... [--folder <path>]...`, `oidc ungroup <group>` | Bir grubun kuralını (§2.1) ekler ya da değiştirir veya kaldırır; `oidc set … [--groups-claim <claim>]` kuralları korur |
| `apeiron web oidc show`, `check`, `remove` | Ayarları gizli anahtar olmadan gösterir; sağlayıcının yanıt verdiğini görmek için ayarlarını ve anahtarlarını okur; kapatır |
| `apeiron web user add <name> --role <role> --oidc <kimlik>` | Sağlayıcı üzerinden, parolasız oturum açan bir kullanıcı ekler |
| `apeiron web user change <name> [--oidc <kimlik>] [--no-password]` | Bir kullanıcıya kimlik verir ya da parolasını kaldırır |
| `apeiron web user second-factor <name> [--off]` | Parola kullanıcısı için ikinci adım: oturum açmak bundan sonra bir doğrulama uygulamasının altı haneli kodunu da ister (TOTP, RFC 6238: HMAC-SHA1, 30 saniye); uygulamaya girilecek gizli anahtarı ve bir `otpauth://` adresini yazar |
| `code` ile `POST /api/login` | Böyle bir kullanıcı için tek başına doğru parola `"second_factor": true` ile yanıtlanır ve oturum başlatmaz; sayfalar o zaman kodu sorar. Saatlerin farklı olabilmesi için kod, geçerli adımdan bir önceki ve bir sonraki adımda da kabul edilir, ama yalnızca bir kez; yanlış bir kod, bekleme için yanlış bir parola sayılır (WEB.tr.md). Gizli anahtar, yalnızca sahibinin okuyabildiği `web.json` dosyasında tutulur |

## 5. Denetim kaydında

| Olay | Ayrıntı |
| --- | --- |
| `sign_in` | `"via": "oidc"`, belirteçteki kimlik ve başarısız olursa nedeni; işe yaradıysa kullanıcı |
| `oidc_set`, `oidc_remove` | Ayarlar, gizli anahtar asla |
| `oidc_group`, `oidc_ungroup` | Kural ya da kuralı kaldırılan grup |
| `user_add`, `user_change` | Bir gruptan oluşturulan ya da oturum açarken kuralına uydurulan bir kullanıcı, grubuyla birlikte; bunu yapan olarak hiçbir kullanıcı yazılmaz |

## 6. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Doğrudan LDAP ve Active Directory | Planlanmadı; Keycloak ve Entra ID gibi sağlayıcılar bir dizini OpenID Connect'in arkasına koyar |
| SAML | Sonra, bir pilot gerektirirse |

## 7. Crate'ler

| Crate | Görevi | Durum |
| --- | --- | --- |
| `apeiron-server` | Sağlayıcı ayarları, oturum açma akışı, belirteç denetimleri, oturum açma ekranı | Tamamlandı |
| `apeiron-cli` | `apeiron web oidc`, kullanıcılar için `--oidc` ve `--no-password` | Tamamlandı |
