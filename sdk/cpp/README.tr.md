> **English:** [README.md](README.md)

# C++ için Apeiron

Apeiron'un C ABI'si üzerinde, C++17 için tek bir başlık dosyası ([docs/SDK.tr.md](../../docs/SDK.tr.md)). Tutamaçlar kendini serbest bırakır, her hata türü bir istisna sınıfıdır. İstekler ve sonuçlar JSON metnidir; bu yüzden bir program istediği JSON kütüphanesini kullanabilir.

Kütüphaneyi `cargo build --release -p apeiron-ffi-rs` ile derleyin, ardından iki başlık klasörüyle derleyip bağlayın:

```bash
c++ -std=c++17 -I crates/apeiron-ffi/include -I sdk/cpp/include program.cpp -L target/release -lapeiron
```

Testler: aynı biçimde derlenen `sdk/cpp/tests/run.cpp`, argüman olarak `sdk/conformance` ile.
