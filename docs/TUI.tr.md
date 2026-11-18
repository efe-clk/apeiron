> **English:** [TUI.md](TUI.md)

# Terminal Arayüzü

> **Durum: Tamamlandı.** `apeiron tui`, masaüstü arayüzünün ekranlarını ([DESKTOP.tr.md](DESKTOP.tr.md) §2) bir terminalin içinde gösterir; ekranı olmayan sunucular ve SSH bağlantıları içindir. Arkasındaki karar ADR-053'tür ([DECISIONS.tr.md](DECISIONS.tr.md)).

## 1. Nasıl kurulu

| Parça | Ne olduğu | Durum |
| --- | --- | --- |
| `apeiron-tui` | Ekranları ratatui ile çizen, tuşları crossterm ile okuyan bir kütüphane crate'i | Tamamlandı |
| `apeiron tui` | Onu terminalde başlatır; verilmişse `--repo` deposu açık olur | Tamamlandı |
| İşlemler | Herkese açık API'nin adla çağrılan işlemleri (SDK.tr.md §2.1), sayfaların çağırdıklarının aynısı; böylece her ekran sayfasının yaptığını yapar | Tamamlandı |
| Metinler | Sayfaların sözlüğü (`crates/apeiron-server/web/i18n.js`), program başlarken okunur; `LC_ALL`, `LC_MESSAGES` ya da `LANG`'e göre İngilizce veya Türkçe | Tamamlandı |

Sunucu ve port yoktur: program API'yi doğrudan çağırır; uzun işlemler, ilerlemesi saniyede birkaç kez çizilen işler olarak çalışır.

## 2. Kullanıcının yapabildikleri

Masaüstü arayüzünün her ekranı buradadır, aynı sözcüklerle.

| Ekran | Ne yapar | Durum |
| --- | --- | --- |
| Başlangıç | Bir depoyu açar (şifreliyse parolasını sorar) ya da oluşturur; yeni kurtarma anahtarını bir kez gösterir | Tamamlandı |
| Genel bakış | Yedekler; her biri klasörü ve en yeni sürümünün zamanıyla | Tamamlandı |
| Yedekle | Bir klasörü `backup/<klasör adı>` ya da kullanıcının verdiği ada, notla, ilerleme ve iptalle yedekler | Tamamlandı |
| Geçmiş | Bir yedeğin sürümleri, en yenisi önce | Tamamlandı |
| Göz at | Bir sürümün içindeki klasörler ve dosyalar | Tamamlandı |
| Geri yükle | Bir sürümü ya da içindeki bir dosyayı veya klasörü yeni bir klasöre | Tamamlandı |
| Karşılaştır | Önceki sürüme göre ne değiştiği, metin dosyalarının değişen satırlarıyla | Tamamlandı |
| Denetle | Bütün depoyu denetler | Tamamlandı |
| Senkron | Uzak hedefler: listeleme, ekleme, kaldırma; iki yönde ya da tek yönde bir senkron turu veya deneme; çakışma kopyaları ve iki sürümden birini tutma | Tamamlandı |
| Parolalar | Anahtarları listeler, parola ya da kurtarma anahtarı ekler, bir anahtarı kaldırır ama sonuncuyu asla | Tamamlandı |
| Eklentiler | Eklentileri listeler, bir `.wasm` dosyasından kurar ve kaldırır | Tamamlandı |
| Zamanlama | Arka plan hizmetinin işleri, son ve sonraki çalışmalarıyla; iş dosyası kullanıcının düzenleyicisinde açılır ve yalnızca geçerliyse kaydedilir | Tamamlandı |
| İncelemeler | İncelemeleri listeler, yenisini ister ve birini adımları ve değişiklikleriyle gösterir; yorum, onay, değişiklik isteme, kapatma ve birleştirme | Tamamlandı |
| Saklama | Saklama kurallarını listeler ve bir uyarıdan sonra yenisini ekler | Tamamlandı |
| Rapor | Bir uyumluluk raporu; istenirse denetim ve uzak hedeflerle | Tamamlandı |

Klasörler terminalin içindeki bir klasör gezgininde seçilir ya da yazılır.

## 3. Tuşlar

| Tuş | Ne yapar |
| --- | --- |
| `Tab`, `Shift+Tab` | Menüde sonraki ya da önceki ekran; bir formun içinde sonraki ya da önceki alan |
| `↑`, `↓`, `PgUp`, `PgDn` | Listede gezinir ya da kaydırır |
| `Enter` | Seçili satırı açar ya da seçili düğmeye basar |
| Köşeli parantezdeki harf | Bir düğmenin işi, örneğin geri yükleme için `[r]` |
| `Esc` | Geri döner ya da çalışan bir işlemi durdurur |
| `Ctrl+C` | Programdan çıkar; çalışan işlem önce durdurulur |

Geri alınamayan sorular, örneğin bir anahtarı kaldırmak ya da saklama kuralı eklemek, `y` tuşunu bekler.

## 4. Bu işin parçası olmayanlar

| Konu | Neden şimdi değil |
| --- | --- |
| Web arayüzünün ekranları (giriş, kullanıcılar, kiracılar, bağlantılar, etkinlik) | Çok kullanıcılı bir sunucuya aittirler; onları `apeiron web` ve komutları yönetir |
| Fare | Her işin bir tuşu vardır; SSH üzerinden terminaller çoğu zaman fareyi iletmez |
| Kullanıcının seçtiği renkler | Terminalin kendi renkleri kullanılır |
