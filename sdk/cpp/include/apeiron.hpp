// The C++ SDK of Apeiron (docs/SDK.md): a thin layer over the C ABI, with
// handles that free themselves and one exception class per kind of error.
// C++17. Requests and results are JSON text in UTF-8, as at the C ABI, so a
// program can use any JSON library; json_string() quotes a string for a request.

#ifndef APEIRON_HPP
#define APEIRON_HPP

#include <chrono>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <optional>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>

#include "apeiron.h"

namespace apeiron {

/// Any error from Apeiron; what() is its message.
class Error : public std::runtime_error {
public:
    Error(int code, std::string kind, const std::string& message, std::string path)
        : std::runtime_error(message), code_(code), kind_(std::move(kind)), path_(std::move(path)) {}
    /// The stable number of the error (docs/API.md §8.6).
    int code() const noexcept { return code_; }
    /// Its kind, such as "not_found".
    const std::string& kind() const noexcept { return kind_; }
    /// The path it is about; empty if none.
    const std::string& path() const noexcept { return path_; }

private:
    int code_;
    std::string kind_;
    std::string path_;
};

#define APEIRON_ERROR(name)      \
    class name : public Error {  \
    public:                      \
        using Error::Error;      \
    }
APEIRON_ERROR(IoError);
APEIRON_ERROR(NotFoundError);
APEIRON_ERROR(AlreadyExistsError);
APEIRON_ERROR(InvalidInputError);
APEIRON_ERROR(AmbiguousError);
APEIRON_ERROR(CorruptError);
APEIRON_ERROR(LockedError);
APEIRON_ERROR(ConflictError);
APEIRON_ERROR(CancelledError);
APEIRON_ERROR(UnsupportedError);
APEIRON_ERROR(InternalError);
APEIRON_ERROR(PasswordError);
#undef APEIRON_ERROR

/// A string as a JSON value, quoted, for building a request.
inline std::string json_string(std::string_view text) {
    std::string out = "\"";
    for (char c : text) {
        if (c == '"' || c == '\\') {
            out += '\\';
            out += c;
        } else if (static_cast<unsigned char>(c) < 0x20) {
            char escaped[7];
            std::snprintf(escaped, sizeof escaped, "\\u%04x", static_cast<unsigned>(c));
            out += escaped;
        } else {
            out += c;
        }
    }
    out += '"';
    return out;
}

namespace detail {

/// The text of a string field of the error JSON the library gives; empty if
/// the field is null. A quote inside a value is always escaped there, so the
/// search cannot stop inside another field.
inline std::string error_field(std::string_view json, std::string_view name) {
    std::string key = "\"" + std::string(name) + "\":\"";
    auto at = json.find(key);
    if (at == std::string_view::npos) {
        return {};
    }
    std::string out;
    for (auto i = at + key.size(); i < json.size() && json[i] != '"'; ++i) {
        if (json[i] != '\\' || i + 1 == json.size()) {
            out += json[i];
            continue;
        }
        char c = json[++i];
        switch (c) {
        case 'n': out += '\n'; break;
        case 't': out += '\t'; break;
        case 'r': out += '\r'; break;
        case 'b': out += '\b'; break;
        case 'f': out += '\f'; break;
        case 'u':
            // The library escapes only control characters this way.
            if (i + 4 < json.size()) {
                out += static_cast<char>(std::stoi(std::string(json.substr(i + 1, 4)), nullptr, 16));
                i += 4;
            }
            break;
        default: out += c;
        }
    }
    return out;
}

[[noreturn]] inline void raise(int code) {
    const char* last = apeiron_last_error();
    std::string json = last != nullptr ? last : "";
    std::string kind = error_field(json, "kind");
    std::string message = error_field(json, "message");
    std::string path = error_field(json, "path");
    if (kind == "io") throw IoError(code, kind, message, path);
    if (kind == "not_found") throw NotFoundError(code, kind, message, path);
    if (kind == "already_exists") throw AlreadyExistsError(code, kind, message, path);
    if (kind == "invalid_input") throw InvalidInputError(code, kind, message, path);
    if (kind == "ambiguous") throw AmbiguousError(code, kind, message, path);
    if (kind == "corrupt") throw CorruptError(code, kind, message, path);
    if (kind == "locked") throw LockedError(code, kind, message, path);
    if (kind == "conflict") throw ConflictError(code, kind, message, path);
    if (kind == "cancelled") throw CancelledError(code, kind, message, path);
    if (kind == "unsupported") throw UnsupportedError(code, kind, message, path);
    if (kind == "internal") throw InternalError(code, kind, message, path);
    if (kind == "key") throw PasswordError(code, kind, message, path);
    throw Error(code, kind, message, path);
}

inline void check(int32_t code) {
    if (code != 0) {
        raise(code);
    }
}

/// Takes a string the library gave.
inline std::string take(char* s) {
    if (s == nullptr) {
        return {};
    }
    std::string out = s;
    apeiron_string_free(s);
    return out;
}

/// The ABI this SDK needs: the same major version, this minor or a later one.
inline void check_abi() {
    constexpr uint32_t major = 0;
    constexpr uint32_t minor = 2;
    uint32_t found = apeiron_abi_version();
    if ((found >> 16) != major || (found & 0xFFFF) < minor) {
        throw UnsupportedError(10, "unsupported",
                               "this SDK needs ABI 0.2 or a later 0.x; the library has " +
                                   std::to_string(found >> 16) + "." +
                                   std::to_string(found & 0xFFFF),
                               "");
    }
}

/// What every handle shares: it owns one pointer, and it can move but not copy.
template <class T, void (*Free)(T*)>
class Handle {
public:
    explicit Handle(T* raw = nullptr) noexcept : raw_(raw) {}
    Handle(Handle&& other) noexcept : raw_(std::exchange(other.raw_, nullptr)) {}
    Handle& operator=(Handle&& other) noexcept {
        if (this != &other) {
            Free(raw_);
            raw_ = std::exchange(other.raw_, nullptr);
        }
        return *this;
    }
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    ~Handle() { Free(raw_); }
    T* get() const noexcept { return raw_; }

private:
    T* raw_;
};

} // namespace detail

/// A running long operation. Freeing it cancels it if it still runs. Use it
/// from one thread at a time.
class Job {
public:
    explicit Job(apeiron_job* raw) noexcept : job_(raw) {}

    /// The current progress as JSON.
    std::string progress() const {
        char* out = nullptr;
        detail::check(apeiron_job_progress(job_.get(), &out));
        return detail::take(out);
    }

    /// Asks the job to stop; wait() then throws CancelledError.
    void cancel() { detail::check(apeiron_job_cancel(job_.get())); }

    /// The result as JSON, or nothing if `timeout` passes first. The job's
    /// result or error is given once.
    std::optional<std::string> wait(std::chrono::milliseconds timeout) {
        return wait_for(timeout.count());
    }

    /// Waits as long as it takes and gives the result as JSON.
    std::string result() { return wait_for(-1).value_or(std::string()); }

    /// Calls `on_progress` with the progress as JSON every `interval` until the
    /// job ends, then gives the result.
    template <class F>
    std::string watch(F&& on_progress,
                      std::chrono::milliseconds interval = std::chrono::milliseconds(200)) {
        for (;;) {
            if (auto done = wait(interval)) {
                return *std::move(done);
            }
            on_progress(progress());
        }
    }

private:
    std::optional<std::string> wait_for(int64_t timeout_ms) {
        char* out = nullptr;
        int32_t code = apeiron_job_wait(job_.get(), timeout_ms, &out);
        if (code == APEIRON_PENDING) {
            return std::nullopt;
        }
        detail::check(code);
        return detail::take(out);
    }

    detail::Handle<apeiron_job, apeiron_job_free> job_;
};

/// A file of a snapshot, read piece by piece (docs/SDK.md §2.3). Use it from
/// one thread at a time.
class FileReader {
public:
    explicit FileReader(apeiron_reader* raw) noexcept : reader_(raw) {}

    /// Reads up to `capacity` bytes and gives how many; 0 is the end of the
    /// file. After an error, every later read throws too.
    std::size_t read(void* buffer, std::size_t capacity) {
        uintptr_t got = 0;
        detail::check(apeiron_reader_read(reader_.get(), static_cast<uint8_t*>(buffer), capacity, &got));
        return static_cast<std::size_t>(got);
    }

private:
    detail::Handle<apeiron_reader, apeiron_reader_free> reader_;
};

/// A file being added to a backup (docs/SDK.md §2.3). Only finish() stores it:
/// a writer destroyed before that stores nothing, as when an exception leaves
/// its scope. Other writes to the repository wait until it is finished or
/// destroyed, so the thread holding it must not start one.
class FileWriter {
public:
    explicit FileWriter(apeiron_writer* raw) noexcept : writer_(raw) {}

    void write(const void* data, std::size_t length) {
        detail::check(apeiron_writer_write(writer_.get(), static_cast<const uint8_t*>(data), length));
    }
    void write(std::string_view data) { write(data.data(), data.size()); }

    /// Ends the file and stores it; gives the snapshot, path and size as JSON.
    std::string finish() {
        char* out = nullptr;
        detail::check(apeiron_writer_finish(writer_.get(), &out));
        return detail::take(out);
    }

private:
    detail::Handle<apeiron_writer, apeiron_writer_free> writer_;
};

/// An open repository; it closes itself. It can be used from several threads
/// at once. Paths are UTF-8.
class Repository {
public:
    /// Creates a repository in a new or empty folder and opens it.
    static Repository init(const std::string& path) {
        detail::check_abi();
        apeiron_repo* raw = nullptr;
        detail::check(apeiron_repo_init(path.c_str(), &raw));
        return Repository(raw);
    }

    /// Creates an encrypted repository; also gives its recovery key.
    static std::pair<Repository, std::string> init_encrypted(const std::string& path,
                                                             const std::string& password) {
        detail::check_abi();
        apeiron_repo* raw = nullptr;
        char* key = nullptr;
        detail::check(apeiron_repo_init_encrypted(path.c_str(), password.c_str(), &raw, &key));
        Repository repo(raw);
        return {std::move(repo), detail::take(key)};
    }

    static Repository open(const std::string& path) {
        detail::check_abi();
        apeiron_repo* raw = nullptr;
        detail::check(apeiron_repo_open(path.c_str(), &raw));
        return Repository(raw);
    }

    /// Opens an encrypted repository with a password or its recovery key.
    static Repository open(const std::string& path, const std::string& secret) {
        detail::check_abi();
        apeiron_repo* raw = nullptr;
        detail::check(apeiron_repo_unlock(path.c_str(), secret.c_str(), &raw));
        return Repository(raw);
    }

    /// Runs a short operation (docs/SDK.md §2.1) and gives its result as JSON.
    std::string call(const std::string& operation, const std::string& request = "{}") const {
        char* out = nullptr;
        detail::check(apeiron_call(repo_.get(), operation.c_str(), request.c_str(), &out));
        return detail::take(out);
    }

    /// Starts a long operation (docs/SDK.md §2.1).
    Job start(const std::string& operation, const std::string& request = "{}") const {
        apeiron_job* raw = nullptr;
        detail::check(apeiron_start(repo_.get(), operation.c_str(), request.c_str(), &raw));
        return Job(raw);
    }

    /// Opens a file of a snapshot for reading.
    FileReader open_file(const std::string& snapshot, const std::string& path) const {
        std::string request =
            "{\"snapshot\":" + json_string(snapshot) + ",\"path\":" + json_string(path) + "}";
        apeiron_reader* raw = nullptr;
        detail::check(apeiron_reader_open(repo_.get(), request.c_str(), &raw));
        return FileReader(raw);
    }

    /// Starts adding a file to a backup. The request is
    /// {"ref", "folder", "name", "message", "host"}.
    FileWriter add_file(const std::string& request) const {
        apeiron_writer* raw = nullptr;
        detail::check(apeiron_writer_open(repo_.get(), request.c_str(), &raw));
        return FileWriter(raw);
    }

private:
    explicit Repository(apeiron_repo* raw) noexcept : repo_(raw) {}
    detail::Handle<apeiron_repo, apeiron_repo_free> repo_;
};

/// Runs a short operation that needs no repository, such as `plugins`.
inline std::string call(const std::string& operation, const std::string& request = "{}") {
    detail::check_abi();
    char* out = nullptr;
    detail::check(apeiron_call(nullptr, operation.c_str(), request.c_str(), &out));
    return detail::take(out);
}

/// Starts `clone`, which makes a new repository from a remote copy.
inline Job clone(const std::string& request) {
    detail::check_abi();
    apeiron_job* raw = nullptr;
    detail::check(apeiron_start(nullptr, "clone", request.c_str(), &raw));
    return Job(raw);
}

} // namespace apeiron

#endif // APEIRON_HPP
