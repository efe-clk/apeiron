# Apeiron for Python

A thin layer over the C ABI of Apeiron ([docs/SDK.md](../../docs/SDK.md)). It needs the native library: build it with `cargo build --release -p apeiron-ffi` and point `APEIRON_LIB` at `target/release/libapeiron.so` (`libapeiron.dylib` on macOS, `apeiron.dll` on Windows), or copy the file next to `apeiron/__init__.py`.

```python
import apeiron

with apeiron.Repository.open("/mnt/backup/apeiron") as repo:
    job = repo.backup("/home/ada/Documents", ref="main")
    for progress in job.watch(interval=0.2):
        print(progress.phase, progress.items_done)
    print(job.result().snapshot)
```

Tests: `APEIRON_LIB=… python3 -m unittest discover -s sdk/python/tests`.
