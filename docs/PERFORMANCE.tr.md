> **English:** [PERFORMANCE.md](PERFORMANCE.md)

# Performans Ölçümleri

> **Durum: Devam ediyor.** [VISION.tr.md](VISION.tr.md) hedefleri Raspberry Pi 4 için yazılmıştır. Bu sayılar çok daha güçlü bir bilgisayardan gelir; programın ne yaptığını gösterir, hedeflerin Raspberry Pi 4'te karşılandığını göstermez. O ölçüm henüz yapılmadı ([ROADMAP.tr.md](ROADMAP.tr.md) §8).

## 1. Nasıl ölçülür

```bash
cargo install --locked --path crates/apeiron-cli
scripts/measure.py ~/.cargo/bin/apeiron /bos/bir/klasor
```

Betik kendi test verisini üretir, her senaryoyu kurulu programla çalıştırır ve bir tablo yazar. Verilen klasörün içinde kendi ev klasörünü kullanır, `--big-gb` değerinin (varsayılan 10) yaklaşık üç katı boş disk ister ve klasörü silmez. `--files` küçük dosya sayısını değiştirir (varsayılan 100000).

## 2. Sonuçlar

2026-11-06'da Linux üzerinde 20 mantıksal işlemci, 16 GB bellek ve NVMe disk ile, sıkıştırılamayan ve tekilleştirilemeyen rastgele veriyle ölçüldü.

| Senaryo | Raspberry Pi 4 hedefi | Önce | Şimdi | Şimdiki en yüksek bellek |
| --- | --- | --- | --- | --- |
| 16 KiB'lık 100000 dosyanın ilk yedeği | — | 23,7 sn | 20,9 sn | 91 MiB |
| Aynı dosyaların değişmeden yedeği | 10 sn altı | 20,4 sn | 0,5 sn | 134 MiB |
| O yedeğin geçmişini okumak | 2 sn altı | 0,1 sn | 0,0 sn | 86 MiB |
| Aralarından adıyla bir dosya bulmak | 2 sn altı | 0,1 sn | 0,1 sn | 86 MiB |
| 10 GiB'lık bir dosya eklemek | 256 MiB bellek altı | 21,0 sn | 20,6 sn | 134 MiB |
| O dosyanın değişmeden yedeği | — | 11,1 sn | 0,1 sn | 121 MiB |
| O dosyayı geri yüklemek | — | 10,4 sn | 10,5 sn | 82 MiB |
| Boşta arka plan hizmeti | 50 MiB bellek altı | — | — | 9 MiB |
| 10 GiB'lık push yarıda kesildikten sonra yeniden gönderilen | En çok bir paket, yaklaşık 32 MiB | 10240 MiB | Kalan kısım: 5101 MiB, 5139 MiB zaten kopyadaydı | 95 MiB |

Apeiron'un `status` komutu yoktur; vizyondaki durum sorgusunu geçmişi okumak ve `sync --dry-run` karşılar, bu yüzden tablo bu okumaları ölçer.

## 3. Ölçümlerin değiştirdikleri

| Bulgu | Değişiklik |
| --- | --- |
| Değişmemiş bir yedek her dosyayı okuyordu | Boyutu ve zamanı son snapshot'la aynı olan ve ondan eski dosya okunmaz ([CLI.tr.md](CLI.tr.md) §4); `--reread` hepsini okur |
| Durdurulan push sıfırdan başlıyordu, çünkü uzak deponun indeksi yalnızca sonda yazılıyordu | Her paketin ardından indeks gelir ve nesneler başvurduklarından sonra gönderilir ([FORMAT.tr.md](FORMAT.tr.md) §15.5) |
| Durdurulan pull sonraki çalışmada kalıcı olarak takılabiliyordu, çünkü indekssiz pakete güveniliyordu | Pull yalnızca indekslerin listelediğine güvenir ([FORMAT.tr.md](FORMAT.tr.md) §15.5) |
| Aktarım bir paketin içinden devam edemez, bu yüzden vizyondaki "en çok 1 chunk" tutamaz | Hedef artık bir pakettir ([DECISIONS.tr.md](DECISIONS.tr.md) ADR-041) |
