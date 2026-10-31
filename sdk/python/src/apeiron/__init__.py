"""Python SDK of Apeiron: a thin layer over its C ABI (docs/SDK.md).

Requests are keyword arguments, results are :class:`Record` values read by
attribute or key, and every error is an :class:`ApeironError` subclass named
after its kind. Long operations return a :class:`Job`.
"""

from __future__ import annotations

import ctypes
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
ABI_MINOR = 1
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
    def clone(url: Any, path: Any, secret: Optional[str] = None) -> Job:
        """Makes a new repository from a remote copy (a job)."""
        return _start(None, "clone", _drop_none({"url": url, "path": path, "secret": secret}))

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

    # Short operations.

    def info(self) -> Record:
        return self.call("info")

    def resolve(self, spec: str) -> str:
        return self.call("resolve", spec=spec)

    def log(self, start: str = "main", limit: int = 50) -> Record:
        return self.call("log", start=start, limit=limit)

    def list(self, snapshot: str, path: str = "") -> list:
        return self.call("list", snapshot=snapshot, path=path)

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

    def push(self, remote: str, ref: str = "main") -> Job:
        return self.start("push", remote=remote, ref=ref)

    def pull(self, remote: str, ref: str = "main") -> Job:
        return self.start("pull", remote=remote, ref=ref)

    def restore_test(self, snapshot: str, **options: Any) -> Job:
        return self.start("restore_test", snapshot=snapshot, **options)

    def search(self, pattern: Optional[str] = None, **options: Any) -> Job:
        return self.start("search", **_drop_none({"pattern": pattern}), **options)

    def sync(self, folder: Any, remote: str, **options: Any) -> Job:
        return self.start("sync", folder=folder, remote=remote, **options)

    def git_import(self, path: Any, **options: Any) -> Job:
        return self.start("git_import", path=path, **options)

    def git_export(self, path: Any, **options: Any) -> Job:
        return self.start("git_export", path=path, **options)
