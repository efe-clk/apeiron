> **English:** [TEXT.md](TEXT.md)

# Metin Eklentisi

> **Durum: Tamamlandı.** Aşama 3'ün 6. işinin tasarımıdır ([ROADMAP.tr.md](ROADMAP.tr.md) §5): ilk gerçek tür sürücüsü ([PLUGIN.tr.md](PLUGIN.tr.md)). Metin dosyalarını satır satır birleştirir ve farklarını gösterir.

Bu eklenti olmadan, bir metin dosyasının farklı satırlarındaki iki değişiklik bir çatışmadır: birleştirme iki sürümü de tutar, birini çatışma kopyası olarak (BRANCH.tr.md §3). Eklentiyle, Git'teki gibi iki değişiklik tek bir dosyaya girer ve `apeiron diff --text` hangi satırların değiştiğini gösterir.

## 1. Kurulum

Eklenti `plugins/text` içinden WebAssembly için derlenir ve her eklenti gibi kurulur:

```bash
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2 --manifest-path plugins/text/Cargo.toml
apeiron plugin add plugins/text/target/wasm32-wasip2/release/apeiron_text.wasm
```

Bundan sonra bu bilgisayardaki her birleştirme ve senkron onu kullanır; `apeiron plugin remove text` bunu durdurur.

## 2. Birleştirme

Eklentiye yalnızca iki taraf bir dosyayı farklı değiştirdiğinde sorulur (PLUGIN.tr.md §4). Her tarafı ortak sürümle satır satır karşılaştırır:

| Durum | Sonuç |
| --- | --- |
| Taraflar farklı satırları değiştirdi | İki değişikliği de içeren tek bir dosya |
| İkisi de aynı değişikliği yaptı | O değişiklik, bir kez |
| İkisi de aynı satırları ya da yan yana satırları farklı değiştirdi | Bir çatışma: eklenti yokmuş gibi çatışma kopyası; aşağıdaki gibi üç sürüm de JSON ya da Markdown değilse |
| Ortak sürüm yok (dosya iki tarafta da eklendi) | İkisi eşit değilse bir çatışma |
| Bir dosya metin değil (UTF-8 değil ya da sıfır baytı var) | Eklenti yokmuş gibi bir çatışma |

Bir çatışma, bir dosyanın içine asla `<<<<<<<` gibi işaretler yazmaz: kullanıcının dosyası geçerli bir dosya olarak kalır ve diğer sürüm yanındadır. Satır sonları olduğu gibi tutulur; satır sonu farklı olan bir satır farklıdır.

Satırlar çakıştığında ortak sürüm ve iki taraf da JSON belgesiyse, eklenti onları bunun yerine yapıya göre birleştirir: nesneleri, her derinlikte, üye üye; bir tarafta eklenen üye tutulur, bir tarafta silinen üye diğer taraf onu değiştirmediyse silinir; sayı, metin ya da dizi gibi diğer her değer yalnızca bir tarafta ya da iki tarafta aynı biçimde değiştirilmiş olmalıdır, yoksa dosya yine bir çatışmadır. Birleşen belge üyeleri bu tarafın sırasıyla, ardından diğer tarafın yeni üyeleriyle tutar ve bu tarafın girintisiyle (hiç yoksa iki boşluk) yazılır; bu yüzden biçimi iki taraftan da farklı olabilir.

Satırlar çakıştığında dosyalar JSON değil de iki taraf da başlıklı Markdown ise (çitli kod dışında bir ile altı `#` ve bir boşlukla başlayan satırlar), eklenti onları bölüm bölüm birleştirir; bölüm, bir başlık ve bir sonrakine kadarki satırlardır, ilk başlıktan önceki metin de bir bölüm sayılır. Bölümler başlık satırlarıyla eşlenir: bir tarafta değişen bölüm o taraftan alınır, iki tarafta değişen satır satır birleştirilir ve çakışmamalıdır, bir tarafta silinen bölüm diğer taraf onu değiştirmediyse silinir, yeni bölüm de kendi tarafında hangi bölümün ardındaysa onun ardına gelir; aynı yere eklenen iki yeni bölümden önce diğer tarafınki gelir. Bir tarafta iki eş başlık varsa bölümler ayırt edilemediği için dosya bir çatışmadır. Eklenti dosya adlarını görmez; bu yüzden böyle satırlar taşıyan her metin, `# ` yorumlu bir betik de, bu yolla birleştirilebilir.

## 3. Farkları göstermek

`apeiron diff <snapshot> <snapshot> --text`, değişen yolları `apeiron diff` gibi listeler ve eklentinin işlediği her dosyanın altında, değişen satırları çevrelerindeki üçer satırla, `diff -u`'nun yazdığı gibi gösterir. Eklenmiş ya da silinmiş bir dosya boş bir dosyayla karşılaştırılır. Hiçbir eklentinin işlemediği dosyalar ve metin olmayan dosyalar satırsız listelenir.

Masaüstü ve web arayüzlerindeki Karşılaştır sayfası aynı satırları gösterir ve değişen sözcükleri de işaretler: silinen bir satır dizisinin ardından aynı sayıda eklenen satır geliyorsa, her silinen satır aynı sıradaki eklenen satırla eşlenir ve ortak en uzun dizilerinin dışında kalan sözcükler vurgulanır. Sözcük; harf, rakam ve `_` dizisidir; boşluklar ve diğer karakterler tek tek sayılır. Çok uzun satır çiftleri sözcük işareti olmadan gösterilir.

## 4. İşlediği dosyalar

Eklenti dosyaları büyük/küçük harf ayırmadan ada göre işler: düz metin ve belgeler (`*.txt`, `*.md`, `*.rst`, `*.tex`, `*.csv`, …), yapılandırma (`*.json`, `*.toml`, `*.yaml`, `*.ini`, `*.xml`, …), web dosyaları (`*.html`, `*.css`, `*.js`, `*.ts`, `*.svg`), yaygın dillerde kaynak kod (`*.rs`, `*.py`, `*.c`, `*.go`, `*.java`, …), altyazılar ve günlükler. `apeiron plugin list` listenin tamamını gösterir.

## 5. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| YAML anahtarları gibi başka yapılara göre birleştirme | Sonraki eklentiler |
| İndirilecek hazır derlenmiş bir eklenti dosyası | İlk sürümle |

## 6. Parçalar

| Parça | Görev | Durum |
| --- | --- | --- |
| `plugins/text` | Eklenti; satır birleştirme ve birleşik fark `diffy` kütüphanesinden gelir | Tamamlandı |
| `apeiron` | `diff_text` (API.tr.md §8.3) | Tamamlandı |
| `apeiron-cli` | `apeiron diff --text` | Tamamlandı |
