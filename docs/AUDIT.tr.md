> **English:** [AUDIT.md](AUDIT.md)

# Denetim Kaydı

> **Durum: Tamamlandı.** Aşama 4'ün 4. işinin tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §6): web arayüzünde ([WEB.tr.md](WEB.tr.md)) ve kullanıcılarında ([USERS.tr.md](USERS.tr.md)) kimin ne yaptığının, kimsenin fark edilmeden değiştiremeyeceği bir kaydı. Arkasındaki karar ADR-029'dur ([DECISIONS.tr.md](DECISIONS.tr.md)).

Birkaç kişi bir yedek sunucusunu paylaştığında admin, "bu klasörü kim, ne zaman geri yükledi?" ve "biri benim adımla oturum açmayı denedi mi?" sorularını yanıtlayabilmelidir.

## 1. Neler kaydedilir

| Olay | Ne zaman | Ayrıntı |
| --- | --- | --- |
| `sign_in` | Doğru ya da yanlış, her oturum açma | Verilen ad, ya da kimlik sağlayıcısı için kimlikle birlikte `"via": "oidc"` ([IDENTITY.tr.md](IDENTITY.tr.md) §5); parola ya da kod asla. Doğru olduğu hâlde doğrulama kodu da gereken bir parolanın sonucu `code_needed`'dır. Git her istekte oturum açar ([GIT.tr.md](GIT.tr.md) §7): burada yalnızca onun yanlış parolaları `"via": "git"` ile kaydedilir |
| `git_fetch` | `git clone` ya da `git fetch`'e gönderilen her paket | Ref öneki ve istenen ref'ler |
| `git_push` | Her `git push` | Ref öneki ve her ref'in yeni Git kimliği ve hatası; hiçbir ref taşınmadıysa sonuç `conflict`'tir |
| `sign_out` | Oturumu kapatma | |
| `call:<operation>` | `list` ya da `log` gibi her kısa işlem | USERS.tr.md §4'teki denetimlerden sonraki istek; snapshot'lar kimlik olarak, yollar çözülmüş olarak; içindeki bir `password` `null` olarak yazılır |
| `start:<operation>` | Başlatılan her uzun işlem: `backup`, `restore`, `verify`, `search`, `report`, `sync` | Aynısı |
| `done:<operation>` | Uzun bir işlemin sonucu alındığında | İş numarası |
| `"via": "cli"` ile `start:backup`, `done:backup`, `start:restore`, `done:restore` | Bu bilgisayardaki web sunucusunun bir denetim kaydı varsa `apeiron backup` ve `apeiron restore`: deposu bu olan kiracınınki, yoksa sunucunun kendisininki | Depo, klasör ya da sürüm ve hedef; sonunda yeni sürüm ya da geri yüklenen dosyalar ve baytlar; başarısız olduysa sonuç hata türüdür. Başlangıcı kaydedilemeyen komut çalışmaz |
| `cancel` | Bir işi durdurma isteği | İş numarası |
| `share_add`, `share_remove` | Bir paylaşım bağlantısı oluşturmak ya da kaldırmak | Bağlantının kimliği, sürümü, yolu ve bitişi ([SHARE.tr.md](SHARE.tr.md) §6) |
| `shared:list`, `shared:read`, `shared:upload` | Bir paylaşım bağlantısının oturum açmadan her kullanımı; `shared:upload` gönderilen dosyayı adlandırır ([SHARE.tr.md](SHARE.tr.md) §7) | Bağlantının kimliği ve yol; bağlantının gizli kısmı asla |
| `shared:unlock` | Bir paylaşım bağlantısı için girilen her parola | Bağlantının kimliği; parola ya da gizli kısım asla |
| `user_add`, `user_change`, `user_password`, `user_remove` | `apeiron web user` ya da Kullanıcılar sayfasında bir admin | Ad, rol, yedekler, klasörler ve kimlik; parola asla |
| `user_second_factor_on`, `user_second_factor_off` | `apeiron web user second-factor` | Ad; gizli anahtar asla |
| `oidc_set`, `oidc_remove`, `oidc_group`, `oidc_ungroup` | `apeiron web oidc` | Sağlayıcının ayarları; gizli anahtar asla |
| `replicate` | Web sunucusunun bir replikaya yaptığı her push | Uzak depo, ref'ler ve nesneler, bir çatışmanın bıraktığı ref'ler ya da hata ([REPLICATION.tr.md](REPLICATION.tr.md) §7) |
| `schedule_save` | Bir kiracının `admin`'inin iş dosyasını kaydetmesi ([DAEMON.tr.md](DAEMON.tr.md) §3.1) | İşlerin adları ya da geçersiz bir dosyanın hatası |
| `tenant_add`, `tenant_limit`, `tenant_remove` | `apeiron web tenant`, o kiracının kendi kaydında | Kiracının ayarları ([TENANTS.tr.md](TENANTS.tr.md) §5) |

Denetimlerin reddettiği bir işlem hatasıyla kaydedilir; böylece denemeler de görünür. Bir işin durumunu yoklamak, oturum durumu ve klasör seçici kaydedilmez. Masaüstü arayüzü hiçbir şey kaydetmez: yalnızca onu çalıştıran kişiye hizmet eder. CLI'nın diğer komutları kaydedilmez; deponun reflog'u her ref değişikliğini kaydeder.

## 2. Dosya

Kayıt, durum klasöründeki (Linux'ta `~/.local/state/apeiron`) `audit.log` dosyasıdır; her satırda bir JSON nesnesi vardır:

```json
{"seq":12,"time":"2026-10-30T14:03:11Z","user":"ayse","from":"192.168.1.20:51544","event":"start:restore","detail":{"snapshot":"5c1e…","target":"/home/ayse/out"},"outcome":"ok","prev":"9f86…"}
```

| Alan | Anlamı |
| --- | --- |
| `seq` | İlk satırda 1, sonraki her satırda bir fazlası |
| `time` | Olayın UTC zamanı |
| `user` | Oturum açmış kullanıcı ya da CLI için onu çalıştıran sistem kullanıcısı; `apeiron web user`, başarısız oturum açmalar ve paylaşım bağlantısı kullanımları için yoktur |
| `from` | Tarayıcı bağlantısının adresi ve kapısı; bir vekilin arkasında vekilinki; CLI için `cli` |
| `event`, `detail` | §1; `detail` içindeki 256 karakterden uzun dizeler kesilir |
| `outcome` | `ok` ya da hata türü (`not_found`, `key`, `unsupported` …); uzun bir işlem izin verilip başladığında `ok` olarak kaydedilir, sonucu `done:` ile gelir |
| `prev` | Önceki satırın baytlarının SHA-256 özeti, onaltılık; ilk satırda 64 sıfır |

Kısa bir işlem çalıştıktan sonra kaydedilir, böylece hiçbir sonuç kaydedilmeden dışarı çıkmaz; uzun bir işlem başlamadan önce kaydedilir, böylece hiçbir şey kaydedilmeden çalışmaz. Satırlara yalnızca ekleme yapılır. Her biri bir kilitle (kaydın yanındaki `audit.log.lock`) yazılır, böylece sunucu ve `apeiron web user` aynı anda yazabilir ve okuyanlar asla yarım satır görmez; istek sürmeden önce diske aktarılır. Bir satır yazılamazsa istek başarısız olur: kaydedilmeyen hiçbir şey olmaz.

## 3. Denetlemek

Her satır kendinden öncekinin özetini tuttuğu için bir satırı değiştirmek, silmek ya da yerini değiştirmek zinciri o noktadan itibaren bozar. `apeiron web audit verify` bütün kaydı okur, ilk bozuk satırı bildirir ve son satırın özetini yazar. Bu özeti ara sıra bir yere yazmak ya da başka bir yere göndermek, zincirin tek başına yakalayamadığı, baştan sona yeniden yazılmış bir kaydı da ortaya çıkarır.

`audit.log` 16 MiB'ı, yaklaşık 50 000 olayı geçtiğinde, sonraki olay önce onu `audit.log.<seq>` olarak yeniden adlandırır (`<seq>` son olayının 12 basamakla yazılmış numarasıdır) ve yeni bir `audit.log` başlatır. Zincir dosyalar boyunca sürer: yeni dosyanın ilk satırı eskisinin son satırının özetini tutar ve `seq` değeri devam eder. `verify` ve `show` parçaları sırayla tek bir kayıt olarak okur. Eski parçalar artık saklanmaları gerekmediğinde bir arşive taşınabilir ya da silinebilir; o zaman `verify` zinciri kalan en eski parçadan denetler ve hangi olaydan itibaren denetlediğini söyler.

## 4. Okumak

| Nerede | Ne |
| --- | --- |
| `apeiron web audit show [--user <name>] [--limit <n>]` | En yeni olaylar, varsayılan olarak 50 tane, en eskisi önce; JSON için `--json` |
| `apeiron web audit verify` | §3 |
| Etkinlik sayfası | Web arayüzündeki bir `admin` için: en yeni 200 olay, en yenisi önce |
| `GET /api/audit?limit=<n>` | Bir `admin` için aynısı; diğerleri `unsupported` alır |

## 5. Syslog'da bir kopya

`apeiron web --audit-syslog <target>`, her olayı dosyaya yazdıktan sonra bir syslog sunucusuna (`udp://host:port`) ya da yerel bir syslog soketine (Linux'ta `/dev/log`) da gönderir; böylece merkezi bir kayıt hizmeti, sunucunun geri alamayacağı bir kopya tutar. Her olay, `authpriv` tesisi ve `info` önem düzeyiyle bir RFC 5424 iletisidir; uygulama adı `apeiron`, `--tenants` ile `apeiron-<tenant>`, ileti kimliği olayın adı, ileti ise olayın dosyadaki satırının ta kendisidir, `seq` ve `prev` alanlarıyla birlikte; böylece kopya zincirle karşılaştırılabilir.

Kayıt yine dosyadır: bir olay ancak diske yazıldıktan sonra gönderilir ve gönderilemeyen bir olay isteği durdurmaz; sunucu, hata çıktısına olayın `seq` numarasıyla bir uyarı yazar, merkezi hizmetin aldığı `seq` numaralarındaki boşluk da bunu gösterir. UDP iletileri uyarı vermeden kaybedebilir; TCP ya da TLS üzerinden ileten bir syslog hizmetine giden yerel bir soket kaybetmez.

## 6. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Eski parçaları başka bir yerde arşivlemek | Admin'e bırakılır: parçalar, yeniden adlandırıldıktan sonra hiç değişmeyen düz dosyalardır |

## 7. Crate'ler

| Crate | Görevi | Durum |
| --- | --- | --- |
| `apeiron-server` | Kayıt, web kipinde kaydetme, `/api/audit` ve Etkinlik sayfası | Tamamlandı |
| `apeiron-cli` | `apeiron web audit`, kullanıcı değişikliklerini kaydetme | Tamamlandı |
