# CLAUDE.md — Apeiron

Apeiron; sürüm kontrolü, arşivleme, yedekleme, senkronizasyon ve bütünlük denetimini tek bir çekirdekte birleştiren, genel amaçlı bir veri yönetim motorudur. Ürün vizyonu ve tüm gereksinimler `docs/VISION.md` dosyasındadır. Büyük bir karar vermeden önce o belge okunur.

## Proje sahibiyle çalışma

- Proje sahibi programlama dili bilmez; tüm kodu Claude Code yazar. Kodu açıklarken jargon en aza indirilir.
- Yanıtlar Türkçe, kısa ve numaralı adımlarla verilir. Liste en fazla 5 maddedir. Giriş, özet tekrarı ve kapanış cümlesi yoktur.
- Her yanıt tek bir somut sonraki adımla biter.
- Sürekli emir kipi ve duygusal ton kullanılmaz.
- Geri alınamaz işlemlerden (force push, geçmiş yeniden yazma, dosya silme) önce onay alınır.

## Alınan kararlar

| Konu | Karar |
| --- | --- |
| Dil | Mümkün olan her yerde Rust. C yalnızca dışa açılan ABI katmanında. |
| Git uyumu | İçe/dışa aktarma zorunlu. Git ağ protokolü (SSH, smart HTTP) hedef ama şart değil. gitoxide (`gix`) kullanılır. |
| Özet algoritması | BLAKE3 (`blake3` crate) |
| Chunk bölme | FastCDC, ortalama 1 MiB, ayarlanabilir |
| Eklenti ortamı | WebAssembly bileşen modeli, Wasmtime |
| Asenkron çalışma zamanı | Tokio (yalnızca ağ, senkron ve daemon katmanlarında; çekirdek senkron kalır) |
| İlk hedef kullanıcı | Kişisel yedek ve ev sunucusu |
| Lisans | MIT OR Apache-2.0 |

## Mimari ilkeler

1. Katmanlar: arayüzler → herkese açık API → hizmetler → çekirdek → depolama hedefleri. Her katman yalnızca altındakini kullanır.
2. Sürümleme, yedek, arşiv ve senkron aynı nesne modelini (chunk, blob, tree, snapshot, ref) kullanır. Dördü için ayrı motor yazılmaz.
3. Nesneler içerik adreslidir ve değişmezdir. Her yazma işlemi: yaz → `fsync` → ref'i atomik güncelle. Güç kesilirse yarım işlem görünmez.
4. Çekirdek hiçbir veri türünü (kod, ses, metin) özel olarak tanımaz. Alana özgü davranış eklentilerdedir.
5. CLI ve diğer arayüzler yalnızca herkese açık API'yi kullanır. Gizli iç kapı yoktur.

## Kod kuralları

- Cargo workspace; her katman ayrı crate (`crates/` altında).
- `unsafe` yalnızca FFI crate'inde ve işletim sistemi çağrılarını tutan küçük `apeiron-sys` crate'inde (ADR-014). Diğer crate'lerde `#![forbid(unsafe_code)]`.
- Her değişiklikten sonra `cargo fmt`, `cargo clippy --all-targets -- -D warnings` ve `cargo test` temiz geçmelidir.
- Kod, tanımlayıcılar, kod yorumları ve commit mesajları İngilizce (açık kaynak katkısı için). Belgeler iki dillidir: X.md İngilizce, X.tr.md Türkçe. Bir belge değişince iki sürüm aynı commit'te güncellenir.
- Commit mesajları Conventional Commits biçiminde (`feat:`, `fix:`, `docs:` ...).
- Depolama formatını etkileyen her değişiklik önce `docs/FORMAT.md` içinde belgelenir, sonra kodlanır.
- Hata türleri tiplidir (`thiserror`); kütüphane crate'lerinde `unwrap`/`expect` kullanılmaz.
- Veri bütünlüğü kodu (yazma, ref güncelleme, kurtarma) hata enjeksiyonu testleriyle doğrulanır.

## Mevcut durum

**Aşama 0 — Format ve çekirdek.** Tamamlandı (2026-10-28):

1. ~~`docs/FORMAT.md`: nesne modeli ve disk formatı spesifikasyonu.~~ Tamamlandı.
2. ~~Cargo workspace iskeleti ve CI (fmt, clippy, test).~~ Tamamlandı.
3. ~~Depolama motoru: chunk yazma/okuma, BLAKE3 doğrulama, atomik ref güncelleme.~~ Tamamlandı.
4. ~~Güç kesintisi ve bozulma testleri.~~ Tamamlandı.
5. ~~Kanonik CBOR kodlaması ve Rust API taslağı.~~ Tamamlandı.

**Aşama 1 — Yerel MVP.** Tamamlandı (2026-10-28). Bir aylık kullanım denemesi proje sahibinin kararıyla atlandı.

**Aşama 2 — Uzak ve senkron.** Devam ediyor. İşler `docs/ROADMAP.md` §4'te, sırasıyla:

1. ~~İstemci tarafı şifreleme (`init --encrypt`, parola ve kurtarma anahtarı).~~ Tamamlandı (2026-10-28).
2. ~~Uzak hedefler (klasör, SFTP, S3), push ve pull.~~ Tamamlandı (2026-10-29).
3. ~~Arka plan hizmeti, zamanlanmış yedek, geri yükleme testi.~~ Tamamlandı (2026-10-29).
4. ~~Senkron ve çatışma tespiti.~~ Tamamlandı (2026-10-29). Aşamanın çıkış ölçütü olan bir aylık senkron denemesi bekliyor.
