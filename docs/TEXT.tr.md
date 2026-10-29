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
| İkisi de aynı satırları ya da yan yana satırları farklı değiştirdi | Bir çatışma: eklenti yokmuş gibi çatışma kopyası |
| Ortak sürüm yok (dosya iki tarafta da eklendi) | İkisi eşit değilse bir çatışma |
| Bir dosya metin değil (UTF-8 değil ya da sıfır baytı var) | Eklenti yokmuş gibi bir çatışma |

Bir çatışma, bir dosyanın içine asla `<<<<<<<` gibi işaretler yazmaz: kullanıcının dosyası geçerli bir dosya olarak kalır ve diğer sürüm yanındadır. Satır sonları olduğu gibi tutulur; satır sonu farklı olan bir satır farklıdır.

## 3. Farkları göstermek

`apeiron diff <snapshot> <snapshot> --text`, değişen yolları `apeiron diff` gibi listeler ve eklentinin işlediği her dosyanın altında, değişen satırları çevrelerindeki üçer satırla, `diff -u`'nun yazdığı gibi gösterir. Eklenmiş ya da silinmiş bir dosya boş bir dosyayla karşılaştırılır. Hiçbir eklentinin işlemediği dosyalar ve metin olmayan dosyalar satırsız listelenir.

## 4. İşlediği dosyalar

Eklenti dosyaları büyük/küçük harf ayırmadan ada göre işler: düz metin ve belgeler (`*.txt`, `*.md`, `*.rst`, `*.tex`, `*.csv`, …), yapılandırma (`*.json`, `*.toml`, `*.yaml`, `*.ini`, `*.xml`, …), web dosyaları (`*.html`, `*.css`, `*.js`, `*.ts`, `*.svg`), yaygın dillerde kaynak kod (`*.rs`, `*.py`, `*.c`, `*.go`, `*.java`, …), altyazılar ve günlükler. `apeiron plugin list` listenin tamamını gösterir.

## 5. Bu işin parçası olmayanlar

| Konu | Nerede |
| --- | --- |
| JSON anahtarları ya da Markdown bölümleri gibi yapıya göre birleştirme | Sonraki eklentiler |
| Bir satırın içinde sözcük sözcük farklar | Daha sonra |
| İndirilecek hazır derlenmiş bir eklenti dosyası | İlk sürümle |

## 6. Parçalar

| Parça | Görev | Durum |
| --- | --- | --- |
| `plugins/text` | Eklenti; satır birleştirme ve birleşik fark `diffy` kütüphanesinden gelir | Tamamlandı |
| `apeiron` | `diff_text` (API.tr.md §8.3) | Tamamlandı |
| `apeiron-cli` | `apeiron diff --text` | Tamamlandı |
