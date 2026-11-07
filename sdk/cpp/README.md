> **Türkçe:** [README.tr.md](README.tr.md)

# Apeiron for C++

One header over Apeiron's C ABI ([docs/SDK.md](../../docs/SDK.md)), for C++17. Handles free themselves, and each kind of error is an exception class. Requests and results are JSON text, so a program can use any JSON library.

Build the library with `cargo build --release -p apeiron-ffi-rs`, then compile with both include folders and link it:

```bash
c++ -std=c++17 -I crates/apeiron-ffi/include -I sdk/cpp/include program.cpp -L target/release -lapeiron
```

Tests: `sdk/cpp/tests/run.cpp`, compiled the same way, with `sdk/conformance` as its argument.
