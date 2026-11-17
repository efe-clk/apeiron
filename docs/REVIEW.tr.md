> **English:** [REVIEW.md](REVIEW.md)

# İncelemeler

> **Durum: Tamamlandı.** [ROADMAP.tr.md](ROADMAP.tr.md) §8'in bir işidir: biri bir daldaki değişikliklerin birleştirilmesini ister, başkaları yorum yapar ve onaylar, onaylanan sürüm birleştirilir. [VISION.tr.md](VISION.tr.md)'nin istediği gibi yalnızca kod için değil, her tür dosya için çalışır. Arkasındaki karar ADR-047'dir ([DECISIONS.tr.md](DECISIONS.tr.md)).

Zeynep bir makaleyi `draft/spring` dalında yazar ([BRANCH.tr.md](BRANCH.tr.md)). Sitesinin üretildiği `main`'e girmeden önce editörü onu okuyup evet demelidir.

## 1. İnceleme nedir

| Parça | Ayrıntı |
| --- | --- |
| Kaynak (from) | Değişiklikleri taşıyan ref, genellikle bir dal |
| Hedef (into) | Birleştirileceği ref; varsayılanı `main` |
| Başlık ve metin | 1 ile 200 karakter arası tek satır ve isteğe bağlı, en fazla 10000 karakterlik bir metin |
| Adımlar | Açılış, yorumlar (tümüne ya da tek bir dosyaya), onaylar, değişiklik istekleri, birleştirme ya da kapatma; her biri yazarı ve zamanıyla |
| Kimlik | 8 rastgele hex karakter; tek bir incelemeye denk gelen herhangi bir başlangıcı da çalışır |

İnceleme dalı kopyalamaz, adını tutar; daldaki yeni bir yedek hemen incelemenin parçası olur.

## 2. Kurallar

| Kural | Ayrıntı |
| --- | --- |
| Onay bir sürümedir | Verildiği anda dalın gösterdiği snapshot'ı adlandırır; dalda yeni bir snapshot olunca sayılmaz, biri yeniden onaylar |
| Değişiklik isteği kalır | Aynı kişi onaylayana kadar; her kişinin son kararı sayılır |
| Yazar onaylayamaz | İncelemeyi açan ad onu onaylayamaz |
| Birleştirmeye hazır | Açık, dal hâlâ var, dalın şimdiki hâlinin en az bir onayı var ve bekleyen değişiklik isteği yok |
| Birleştirme | Tam olarak onaylanan snapshot'ı `apeiron merge` gibi birleştirir ([BRANCH.tr.md](BRANCH.tr.md) §2). Birleştirme çakışma kopyaları üretecekse hiçbir şey değişmez ve inceleme açık kalır: önce `into`'yu dala birleştirin, sonra yeniden onay isteyin |
| Birleştirme ya da kapatmadan sonra | Yalnızca yorum alınır; kapatılmış bir inceleme yeniden açılamaz, yenisi açılır |

Adlar `--author` ile verilenlerdir ya da işletim sisteminin kullanıcısıdır. Bir bilgisayarda depoya yazabilen herkes, bir yedeğin yazarında olduğu gibi, istediği adı yazabilir; inceleme birbirine güvenen kişiler arasındaki uzlaşmanın kaydıdır, erişim denetimi değildir. Bir ref'i kimin değiştirebileceğine izinler ([USERS.tr.md](USERS.tr.md)) ve saklama kuralları ([ARCHIVE.tr.md](ARCHIVE.tr.md)) karar verir.

## 3. Komutlar

| Komut | Ne yapar |
| --- | --- |
| `apeiron review open <from> [--into <ref>] --title <text> [--text <text>] [--author <name>]` | Bir inceleme açar ve kimliğini yazdırır |
| `apeiron review list [--all]` | Açık incelemeler, en yenisi önce; `--all` ile birleştirilmiş ve kapatılmış olanlar da |
| `apeiron review show <id>` | Her adım, kimin onayladığı, kimin değişiklik istediği ve bir birleştirmenin `into`'da değiştireceği dosyalar |
| `apeiron review comment <id> <text> [--path <file>] [--author <name>]` | Bir yorum ekler, `--path` ile tek bir dosyaya |
| `apeiron review approve <id> [--text <text>] [--author <name>]` | Dalın şimdiki hâlini onaylar |
| `apeiron review request-changes <id> --text <text> [--author <name>]` | Değişiklik ister |
| `apeiron review close <id> [--text <text>] [--author <name>]` | Birleştirmeden kapatır |
| `apeiron review merge <id> [--message <text>] [--author <name>]` | Hazır bir incelemeyi birleştirir; değilse neyin eksik olduğunu söyleyen bir iletiyle çıkış kodu 1'dir |

```text
$ apeiron review open draft/spring --title "Spring article" --author zeynep
Opened review 3f9a01c2: merge draft/spring into main
$ apeiron review request-changes 3f9a --text "The second part is long" --author editor
$ apeiron backup ~/articles --ref draft/spring
$ apeiron review approve 3f9a --author editor
$ apeiron review merge 3f9a --author zeynep
Merged review 3f9a01c2 into main: 7c2b04e1
```

Aynı adımlar API'nin ([API.tr.md](API.tr.md) §8.3) ve SDK'ların ([SDK.tr.md](SDK.tr.md)) `reviews`, `review`, `open_review`, `note_review` ve `merge_review` işlemleridir. Bir metin dosyasında neyin değiştiğini metin eklentisi gösterir ([TEXT.tr.md](TEXT.tr.md)); komut satırında bunu `apeiron diff --text main draft/spring` yapar.

## 4. İncelemeler nerede tutulur

Her adım `apeiron/reviews` ref'inde, `<id>-<nanosaniye>-<rastgele>.json` adlı bir JSON dosyasıdır; alanları `action`, `author`, `time` (Unix saniyesi) ve adımın gerektirdiği kadarıyla `text`, `path`, `snapshot`, `from`, `into` ve `title`'dır. Yeni bir adım bir dosya ekler, hiçbirini değiştirmez; böylece aynı anda yazan iki kişinin ikisi de adımlarını korur: ref karşılaştır-ve-değiştir ile ilerler ve ikinci yazıcı birincinin üstünde yeniden dener ([BRANCH.tr.md](BRANCH.tr.md) §2). Bir incelemenin adımları önce açılış, sonra dosya adına göre okunur; böylece her kopya onları aynı sırada gösterir.

Bir ref'te durdukları için incelemeler depoyla birlikte şifrelenir, push ve pull ile kopyalanır ve diğer her ref gibi saklama kuralları ve yasal saklama kilitleriyle korunur. Birleştirmenin kendisi de işlem günlüğüne yazılır ([FORMAT.tr.md](FORMAT.tr.md) §9.5).

## 5. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| Web ve masaüstü arayüzlerinde incelemeler | Sonra; [ROADMAP.tr.md](ROADMAP.tr.md) §8 |
| Bir ref değişmeden önce onay şartı | Sonra; bugün inceleme bir uzlaşmadır ve ref'i değiştirebilen herkes onaysız da birleştirebilir |
| Bir dosyanın tek satırına yorum | Sonra; yorum bir dosyayı adlandırır |

## 6. Crate'ler

| Crate | Bölüm |
| --- | --- |
| `apeiron` | `review.rs`: adımlar, kurallar ve birleştirme |
| `apeiron-cli` | `apeiron review` |
