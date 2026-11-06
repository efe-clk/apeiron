> **English:** [README.md](README.md)

# Python için Apeiron

Apeiron'un C ABI'si üzerinde ince bir katman ([docs/SDK.tr.md](../../docs/SDK.tr.md)). Yerel kütüphaneye ihtiyaç duyar: `cargo build --release -p apeiron-ffi` ile derleyin ve `APEIRON_LIB` değişkenini `target/release/libapeiron.so` dosyasına (macOS'ta `libapeiron.dylib`, Windows'ta `apeiron.dll`) yöneltin ya da dosyayı `apeiron/__init__.py` yanına kopyalayın.

```python
import apeiron

with apeiron.Repository.open("/mnt/backup/apeiron") as repo:
    job = repo.backup("/home/ada/Documents", ref="main")
    for progress in job.watch(interval=0.2):
        print(progress.phase, progress.items_done)
    print(job.result().snapshot)
```

Testler: `APEIRON_LIB=… python3 -m unittest discover -s sdk/python/tests`.
