"""Python SDK of Apeiron: a thin layer over its C ABI (docs/SDK.md).

Requests are keyword arguments, results are :class:`Record` values read by
attribute or key, and every error is an :class:`ApeironError` subclass named
after its kind. Long operations return a :class:`Job`.
"""

from __future__ import annotations

import ctypes
import io
import ctypes.util
import json
import os
import sys
import threading
from pathlib import Path
from typing import Any, Iterator, Optional, Tuple

__all__ = [
    "Repository",
    "plugins",
    "add_plugin",
    "remove_plugin",
    "saved_searches",
    "save_search",
    "forget_search",
    "Job",
    "Record",
    "ApeironError",
    "IoError",
    "NotFoundError",
    "AlreadyExistsError",
    "InvalidInputError",
    "AmbiguousError",
    "CorruptError",
    "LockedError",
    "ConflictError",
    "CancelledError",
    "UnsupportedError",
    "InternalError",
    "PasswordError",
]

# The ABI version this SDK is written for (SDK.md §2).
ABI_MAJOR = 0
ABI_MINOR = 2
_PENDING = -1


class Record(dict):
    """A JSON object whose keys can also be read as attributes."""

    def __getattr__(self, name: str) -> Any:
        try:
            return self[name]
        except KeyError:
            raise AttributeError(name) from None


class ApeironError(Exception):
    """An error with a stable code and kind (API.md §8.6)."""

    def __init__(self, code: int, kind: str, message: str, path: Optional[str] = None):
        super().__init__(message)
        self.code = code
        self.kind = kind
        self.message = message
        self.path = path


class IoError(ApeironError):
    pass


class NotFoundError(ApeironError):
    pass


class AlreadyExistsError(ApeironError):
    pass


class InvalidInputError(ApeironError):
    pass


class AmbiguousError(ApeironError):
    pass


class CorruptError(ApeironError):
    pass


class LockedError(ApeironError):
    pass


class ConflictError(ApeironError):
    pass


class CancelledError(ApeironError):
    pass


class UnsupportedError(ApeironError):
    pass


class InternalError(ApeironError):
    pass


class PasswordError(ApeironError):
    """The repository is encrypted and the password or recovery key is missing or wrong."""


_ERRORS = {
    "io": IoError,
    "not_found": NotFoundError,
    "already_exists": AlreadyExistsError,
    "invalid_input": InvalidInputError,
    "ambiguous": AmbiguousError,
    "corrupt": CorruptError,
    "locked": LockedError,
    "conflict": ConflictError,
    "cancelled": CancelledError,
    "unsupported": UnsupportedError,
    "internal": InternalError,
    "key": PasswordError,
}

_lib = None
_lib_lock = threading.Lock()


def _library_name() -> str:
    if sys.platform == "win32":
        return "apeiron.dll"
    if sys.platform == "darwin":
        return "libapeiron.dylib"
    return "libapeiron.so"


def _library() -> ctypes.CDLL:
    """Loads the native library once: from ``APEIRON_LIB``, next to this
    file, or from the system's library path."""
    global _lib
    with _lib_lock:
        if _lib is not None:
            return _lib
        path = os.environ.get("APEIRON_LIB")
        if path:
            path = os.path.abspath(path)
        else:
            beside = Path(__file__).with_name(_library_name())
            path = str(beside) if beside.exists() else ctypes.util.find_library("apeiron")
        if not path:
            raise ImportError(
                "the Apeiron library was not found; set APEIRON_LIB to the path of "
                + _library_name()
            )
        lib = ctypes.CDLL(path)
        p = ctypes.c_void_p
        pp = ctypes.POINTER(ctypes.c_void_p)
        s = ctypes.c_char_p
        i32 = ctypes.c_int32
        for name, args, result in [
            ("apeiron_abi_version", [], ctypes.c_uint32),
            ("apeiron_last_error", [], s),
            ("apeiron_string_free", [p], None),
            ("apeiron_repo_init", [s, pp], i32),
            ("apeiron_repo_init_encrypted", [s, s, pp, pp], i32),
            ("apeiron_repo_open", [s, pp], i32),
            ("apeiron_repo_unlock", [s, s, pp], i32),
            ("apeiron_repo_free", [p], None),
            ("apeiron_call", [p, s, s, pp], i32),
            ("apeiron_start", [p, s, s, pp], i32),
            ("apeiron_job_progress", [p, pp], i32),
            ("apeiron_job_cancel", [p], i32),
            ("apeiron_job_wait", [p, ctypes.c_int64, pp], i32),
            ("apeiron_job_free", [p], None),
            ("apeiron_reader_open", [p, s, pp], i32),
            ("apeiron_reader_read", [p, p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_size_t)], i32),
            ("apeiron_reader_free", [p], None),
            ("apeiron_writer_open", [p, s, pp], i32),
            ("apeiron_writer_write", [p, ctypes.c_char_p, ctypes.c_size_t], i32),
            ("apeiron_writer_finish", [p, pp], i32),
            ("apeiron_writer_free", [p], None),
        ]:
            fn = getattr(lib, name)
            fn.argtypes = args
            fn.restype = result
        version = lib.apeiron_abi_version()
        major, minor = version >> 16, version & 0xFFFF
        if major != ABI_MAJOR or minor < ABI_MINOR:
            raise ImportError(
                f"the Apeiron library at {path} has ABI {major}.{minor}; "
                f"this SDK needs {ABI_MAJOR}.{ABI_MINOR} or a later {ABI_MAJOR}.x"
            )
        _lib = lib
        return lib


def _check(code: int) -> int:
    """Raises the error of a failed call; returns 0 or ``_PENDING``."""
    if code in (0, _PENDING):
        return code
    raw = _library().apeiron_last_error()
    e = json.loads(raw.decode()) if raw else {"kind": "internal", "message": "unknown error"}
    kind = e.get("kind", "internal")
    raise _ERRORS.get(kind, ApeironError)(code, kind, e.get("message", ""), e.get("path"))


def _text(value: Any) -> bytes:
    return os.fspath(value).encode() if isinstance(value, os.PathLike) else str(value).encode()


def _request(request: dict) -> bytes:
    return json.dumps(request, default=os.fspath).encode()


def _take(ptr: ctypes.c_void_p) -> Any:
    """Reads and frees a JSON string returned by the library."""
    lib = _library()
    try:
        return json.loads(ctypes.string_at(ptr.value).decode(), object_hook=Record)
    finally:
        lib.apeiron_string_free(ptr)


def _drop_none(request: dict) -> dict:
    return {k: v for k, v in request.items() if v is not None}


class Job:
    """A running long operation (API.md §8.5). Read its progress, cancel it
    and wait for its result; closing a running job cancels it."""

    def __init__(self, handle: ctypes.c_void_p):
        self._handle = handle
        self._done = False
        self._result: Any = None
        self._error: Optional[ApeironError] = None

    def progress(self) -> Record:
        out = ctypes.c_void_p()
        _check(_library().apeiron_job_progress(self._live(), ctypes.byref(out)))
        return _take(out)

    def cancel(self) -> None:
        if not self._done:
            _check(_library().apeiron_job_cancel(self._live()))

    def wait(self, timeout: Optional[float] = None) -> Any:
        """The result; raises the job's error, or ``TimeoutError`` when
        ``timeout`` seconds pass first. It can be called again."""
        if not self._done:
            lib = _library()
            out = ctypes.c_void_p()
            # Short waits keep Ctrl+C working while waiting without limit.
            step = 200 if timeout is None else max(0, int(timeout * 1000))
            while True:
                try:
                    code = _check(lib.apeiron_job_wait(self._live(), step, ctypes.byref(out)))
                except ApeironError as e:
                    self._done, self._error = True, e
                    break
                if code == 0:
                    self._done, self._result = True, _take(out)
                    break
                if timeout is not None:
                    raise TimeoutError("the job is still running")
        if self._error is not None:
            raise self._error
        return self._result

    def result(self) -> Any:
        return self.wait()

    def watch(self, interval: float = 0.2) -> Iterator[Record]:
        """Yields the progress every ``interval`` seconds until the job ends."""
        while not self._done:
            yield self.progress()
            try:
                self.wait(interval)
            except TimeoutError:
                continue
            except ApeironError:
                return

    def close(self) -> None:
        if self._handle:
            _library().apeiron_job_free(self._handle)
            self._handle = None

    def _live(self) -> ctypes.c_void_p:
        if not self._handle:
            raise InvalidInputError(4, "invalid_input", "the job is closed")
        return self._handle

    def __enter__(self) -> "Job":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass


def _start(handle: Optional[ctypes.c_void_p], operation: str, request: dict) -> Job:
    out = ctypes.c_void_p()
    _check(
        _library().apeiron_start(handle, operation.encode(), _request(request), ctypes.byref(out))
    )
    return Job(out)


def _call_global(operation: str, request: dict) -> Any:
    out = ctypes.c_void_p()
    _check(
        _library().apeiron_call(None, operation.encode(), _request(request), ctypes.byref(out))
    )
    return _take(out)


def plugins(dir: Any = None) -> list:
    """The installed plugins (docs/PLUGIN.md); ``dir`` is the plugin folder,
    the user's by default."""
    return _call_global("plugins", _drop_none({"dir": dir}))


def add_plugin(file: Any, dir: Any = None) -> Record:
    """Checks a plugin (a WebAssembly component) and installs it."""
    return _call_global("plugin_add", _drop_none({"file": file, "dir": dir}))


def remove_plugin(name: str, dir: Any = None) -> Record:
    """Removes an installed plugin."""
    return _call_global("plugin_remove", _drop_none({"name": name, "dir": dir}))


def conflicts(folder: Any) -> list:
    """The conflict copies sync and merge left in ``folder`` (docs/SYNC.md
    §4), each with ``path``, ``exists`` and ``copy``; read ``copy`` as
    ``c["copy"]``, since ``c.copy`` is the method of ``dict``."""
    return _call_global("conflicts", {"folder": folder})


def resolve_conflict(folder: Any, copy: str, keep: str) -> Record:
    """Keeps one version of a conflict copy: ``keep="this"`` deletes the
    copy, ``keep="copy"`` puts it in place of the file at its path."""
    return _call_global("resolve_conflict", {"folder": folder, "copy": copy, "keep": keep})


def saved_searches(file: Any = None) -> list:
    """The searches saved under a name (docs/SEARCH.md §6); ``file`` holds
    them, the user's by default."""
    return _call_global("saved_searches", _drop_none({"file": file}))


def save_search(name: str, pattern: Optional[str] = None, file: Any = None, **options: Any) -> None:
    """Saves a search under ``name``, replacing one of the same name; the
    options are those of :meth:`Repository.search`."""
    request = _drop_none({"name": name, "pattern": pattern, "file": file})
    return _call_global("save_search", {**request, **options})


def forget_search(name: str, file: Any = None) -> Record:
    """Removes a saved search and returns it."""
    return _call_global("forget_search", _drop_none({"name": name, "file": file}))


class FileReader(io.RawIOBase):
    """A file of a snapshot, read piece by piece (docs/SDK.md §2.3)."""

    def __init__(self, handle: ctypes.c_void_p):
        super().__init__()
        self._handle = handle

    def readable(self) -> bool:
        return True

    def readinto(self, buffer: Any) -> int:
        if not self._handle:
            raise ValueError("the file is closed")
        size = len(memoryview(buffer).cast("B"))
        if size == 0:
            return 0
        target = (ctypes.c_char * size).from_buffer(buffer)
        read = ctypes.c_size_t()
        _check(
            _library().apeiron_reader_read(
                self._handle, ctypes.addressof(target), size, ctypes.byref(read)
            )
        )
        return read.value

    def close(self) -> None:
        if getattr(self, "_handle", None):
            _library().apeiron_reader_free(self._handle)
            self._handle = None
        super().close()


class FileWriter:
    """A file being added to a backup (docs/SDK.md §2.3). Only :meth:`finish`
    stores it; leaving a ``with`` block normally finishes it, and an exception
    stores nothing. Other writes to the repository wait until it is closed."""

    def __init__(self, handle: ctypes.c_void_p):
        self._handle = handle
        self.report: Optional[Record] = None

    def write(self, data: bytes) -> int:
        _check(_library().apeiron_writer_write(self._live(), bytes(data), len(data)))
        return len(data)

    def finish(self) -> Record:
        out = ctypes.c_void_p()
        _check(_library().apeiron_writer_finish(self._live(), ctypes.byref(out)))
        self.report = _take(out)
        return self.report

    def close(self) -> None:
        if getattr(self, "_handle", None):
            _library().apeiron_writer_free(self._handle)
            self._handle = None

    def _live(self) -> ctypes.c_void_p:
        if not self._handle:
            raise InvalidInputError(4, "invalid_input", "the file is closed")
        return self._handle

    def __enter__(self) -> "FileWriter":
        return self

    def __exit__(self, kind: Any, *rest: Any) -> None:
        try:
            if kind is None and self.report is None:
                self.finish()
        finally:
            self.close()

    def __del__(self) -> None:
        self.close()


class Repository:
    """An open repository. Use it in a ``with`` block or call :meth:`close`."""

    def __init__(self, handle: ctypes.c_void_p):
        self._handle = handle

    @classmethod
    def init(cls, path: Any) -> "Repository":
        """Creates a repository in a new or empty folder and opens it."""
        out = ctypes.c_void_p()
        _check(_library().apeiron_repo_init(_text(path), ctypes.byref(out)))
        return cls(out)

    @classmethod
    def init_encrypted(cls, path: Any, password: str) -> Tuple["Repository", str]:
        """Creates an encrypted repository; returns it and its recovery key,
        to be shown to the user once."""
        out = ctypes.c_void_p()
        key = ctypes.c_void_p()
        lib = _library()
        _check(
            lib.apeiron_repo_init_encrypted(
                _text(path), password.encode(), ctypes.byref(out), ctypes.byref(key)
            )
        )
        try:
            recovery_key = ctypes.string_at(key.value).decode()
        finally:
            lib.apeiron_string_free(key)
        return cls(out), recovery_key

    @classmethod
    def open(cls, path: Any, password: Optional[str] = None) -> "Repository":
        """Opens a repository; an encrypted one needs its password or recovery key."""
        out = ctypes.c_void_p()
        lib = _library()
        if password is None:
            _check(lib.apeiron_repo_open(_text(path), ctypes.byref(out)))
        else:
            _check(lib.apeiron_repo_unlock(_text(path), password.encode(), ctypes.byref(out)))
        return cls(out)

    @staticmethod
    def clone(url: Any, path: Any, secret: Optional[str] = None, rate: Optional[int] = None) -> Job:
        """Makes a new repository from a remote copy (a job); ``rate`` limits
        the speed in bytes per second (docs/SYNC.md §2)."""
        request = {"url": url, "path": path, "secret": secret, "rate": rate}
        return _start(None, "clone", _drop_none(request))

    def close(self) -> None:
        if self._handle:
            _library().apeiron_repo_free(self._handle)
            self._handle = None

    def __enter__(self) -> "Repository":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:
            pass

    def _live(self) -> ctypes.c_void_p:
        if not self._handle:
            raise InvalidInputError(4, "invalid_input", "the repository is closed")
        return self._handle

    # Any operation by name (SDK.md §2.1).

    def call(self, operation: str, **request: Any) -> Any:
        """Runs a short operation and returns its result."""
        out = ctypes.c_void_p()
        _check(
            _library().apeiron_call(
                self._live(), operation.encode(), _request(request), ctypes.byref(out)
            )
        )
        return _take(out)

    def start(self, operation: str, **request: Any) -> Job:
        """Starts a long operation."""
        return _start(self._live(), operation, request)

    # File content as a stream (docs/SDK.md §2.3).

    def open_file(self, snapshot: str, path: str) -> FileReader:
        """Opens a file of a snapshot for reading."""
        out = ctypes.c_void_p()
        _check(
            _library().apeiron_reader_open(
                self._live(), _request({"snapshot": snapshot, "path": path}), ctypes.byref(out)
            )
        )
        return FileReader(out)

    def add_file(
        self, ref: str, name: str, folder: str = "", message: Optional[str] = None
    ) -> FileWriter:
        """Starts adding a file to a backup, as a new snapshot of ``ref``."""
        request = {"ref": ref, "name": name, "folder": folder, "message": message}
        out = ctypes.c_void_p()
        _check(_library().apeiron_writer_open(self._live(), _request(request), ctypes.byref(out)))
        return FileWriter(out)

    # Short operations.

    def info(self) -> Record:
        return self.call("info")

    def resolve(self, spec: str) -> str:
        return self.call("resolve", spec=spec)

    def log(self, start: str = "main", limit: int = 50, grep: Optional[str] = None) -> Record:
        return self.call("log", start=start, limit=limit, grep=grep)

    def list(self, snapshot: str, path: str = "") -> list:
        return self.call("list", snapshot=snapshot, path=path)

    def duplicates(self, snapshot: str = "main", smallest: int = 0) -> list:
        return self.call("duplicates", snapshot=snapshot, smallest=smallest)

    def diff(self, from_: str, to: str) -> list:
        return self.call("diff", **{"from": from_, "to": to})

    def diff_text(self, from_: str, to: str) -> list:
        """Changed paths with the changed lines a plugin shows (docs/TEXT.md)."""
        return self.call("diff_text", **{"from": from_, "to": to})

    def refs(self) -> list:
        return self.call("refs")

    def reflog(self, name: str) -> list:
        return self.call("reflog", name=name)

    def create_ref(self, name: str, snapshot: str = "main") -> str:
        return self.call("create_ref", name=name, snapshot=snapshot)

    def delete_ref(self, name: str, expected: str) -> None:
        self.call("delete_ref", name=name, expected=expected)

    def merge(self, from_: str, into: str = "main", **options: Any) -> Record:
        return self.call("merge", **{"from": from_, "into": into}, **options)

    def cherry_pick(self, from_: str, into: str = "main", **options: Any) -> Record:
        return self.call("cherry_pick", **{"from": from_, "into": into}, **options)

    def rebase(self, branch: str, onto: str = "main", **options: Any) -> Record:
        return self.call("rebase", **{"from": onto, "into": branch}, **options)

    def prune(self, ref: str = "main", **rules: Any) -> Record:
        return self.call("prune", ref=ref, **rules)

    def repair(self) -> Record:
        return self.call("repair")

    def keys(self) -> list:
        return self.call("keys")

    def add_password(self, password: str) -> Record:
        return self.call("add_password", password=password)

    def add_recovery_key(self) -> Record:
        return self.call("add_recovery_key")

    def remove_key(self, id: str) -> str:
        return self.call("remove_key", id=id)

    def remotes(self) -> list:
        return self.call("remotes")

    def add_remote(self, name: str, url: str) -> Record:
        return self.call("add_remote", name=name, url=url)

    def remove_remote(self, name: str) -> None:
        self.call("remove_remote", name=name)

    def retention(self) -> list:
        return self.call("retention")

    def add_retention(self, refs: str, days: int) -> Record:
        return self.call("add_retention", refs=refs, days=days)

    def holds(self) -> list:
        return self.call("holds")

    def add_hold(self, refs: str, reason: str) -> Record:
        return self.call("add_hold", refs=refs, reason=reason)

    def lift_hold(self, id: str, reason: str) -> Record:
        return self.call("lift_hold", id=id, reason=reason)

    # Long operations; each returns a Job.

    def backup(self, source: Any, ref: str = "main", **options: Any) -> Job:
        return self.start("backup", source=source, ref=ref, **options)

    def restore(self, snapshot: str, target: Any, **options: Any) -> Job:
        return self.start("restore", snapshot=snapshot, target=target, **options)

    def export(self, snapshot: str, output: Any, **options: Any) -> Job:
        return self.start("export", snapshot=snapshot, output=output, **options)

    def verify(self) -> Job:
        return self.start("verify")

    def gc(self, **options: Any) -> Job:
        return self.start("gc", **options)

    def gc_remote(self, remote: str, **options: Any) -> Job:
        return self.start("gc_remote", remote=remote, **options)

    def push(
        self, remote: str, ref: str = "main", rate: Optional[int] = None, replace: bool = False
    ) -> Job:
        request = {"remote": remote, "ref": ref, "rate": rate, "replace": replace}
        return self.start("push", **_drop_none(request))

    def pull(self, remote: str, ref: str = "main", rate: Optional[int] = None) -> Job:
        return self.start("pull", **_drop_none({"remote": remote, "ref": ref, "rate": rate}))

    def import_archive(self, archive: Any, ref: str = "main", **options: Any) -> Job:
        return self.start("import_archive", archive=archive, ref=ref, **options)

    def push_all(self, remote: str, rate: Optional[int] = None) -> Job:
        return self.start("push_all", **_drop_none({"remote": remote, "rate": rate}))

    def pull_all(self, remote: str, rate: Optional[int] = None) -> Job:
        return self.start("pull_all", **_drop_none({"remote": remote, "rate": rate}))

    def repair_from(self, remote: str, rate: Optional[int] = None) -> Job:
        return self.start("repair_from", **_drop_none({"remote": remote, "rate": rate}))

    def report(self, **options: Any) -> Job:
        return self.start("report", **options)

    def restore_test(self, snapshot: str, **options: Any) -> Job:
        return self.start("restore_test", snapshot=snapshot, **options)

    def search(self, pattern: Optional[str] = None, **options: Any) -> Job:
        return self.start("search", **_drop_none({"pattern": pattern}), **options)

    def sync(self, folder: Any, remote: str, **options: Any) -> Job:
        return self.start("sync", folder=folder, remote=remote, **options)

    def switch(self, snapshot: str, folder: Any, **options: Any) -> Job:
        return self.start("switch", snapshot=snapshot, folder=folder, **options)

    def git_import(self, path: Any, **options: Any) -> Job:
        return self.start("git_import", path=path, **options)

    def git_export(self, path: Any, **options: Any) -> Job:
        return self.start("git_export", path=path, **options)
