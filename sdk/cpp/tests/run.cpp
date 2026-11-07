// The shared conformance scenarios (docs/SDK.md §6) and what only C++ has, run
// through the C++ SDK. Usage: run <folder of the scenarios>
//
// The small JSON reader below belongs to this test: the SDK itself passes JSON
// as text and needs none.

#include <algorithm>
#include <apeiron.hpp>
#include <cmath>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <iterator>
#include <map>
#include <random>
#include <sstream>
#include <type_traits>
#include <vector>

namespace fs = std::filesystem;

namespace {

struct Json {
    enum Type { Null, Bool, Number, String, Array, Object } type = Null;
    bool boolean = false;
    double number = 0;
    std::string text;
    std::vector<Json> items;
    std::map<std::string, Json> fields;

    const Json* get(const std::string& name) const {
        auto at = fields.find(name);
        return at == fields.end() ? nullptr : &at->second;
    }
};

class Parser {
public:
    explicit Parser(const std::string& text) : s_(text) {}

    Json parse() {
        Json value = next();
        space();
        if (i_ != s_.size()) fail();
        return value;
    }

private:
    [[noreturn]] void fail() const { throw std::runtime_error("bad JSON at " + std::to_string(i_) + ": " + s_); }
    void space() {
        while (i_ < s_.size() && std::isspace(static_cast<unsigned char>(s_[i_]))) ++i_;
    }
    bool take(const char* word) {
        std::string_view w(word);
        if (s_.compare(i_, w.size(), w) != 0) return false;
        i_ += w.size();
        return true;
    }
    Json next() {
        space();
        if (i_ >= s_.size()) fail();
        Json v;
        char c = s_[i_];
        if (c == '{') {
            v.type = Json::Object;
            ++i_;
            space();
            if (take("}")) return v;
            for (;;) {
                space();
                std::string key = string();
                space();
                if (!take(":")) fail();
                v.fields[key] = next();
                space();
                if (take("}")) return v;
                if (!take(",")) fail();
            }
        }
        if (c == '[') {
            v.type = Json::Array;
            ++i_;
            space();
            if (take("]")) return v;
            for (;;) {
                v.items.push_back(next());
                space();
                if (take("]")) return v;
                if (!take(",")) fail();
            }
        }
        if (c == '"') {
            v.type = Json::String;
            v.text = string();
            return v;
        }
        if (take("null")) return v;
        if (take("true") || take("false")) {
            v.type = Json::Bool;
            v.boolean = s_[i_ - 1] == 'e' && s_[i_ - 2] == 'u';
            return v;
        }
        std::size_t used = 0;
        v.type = Json::Number;
        try {
            v.number = std::stod(s_.substr(i_), &used);
        } catch (const std::exception&) {
            fail();
        }
        i_ += used;
        return v;
    }
    void utf8(unsigned code, std::string& out) {
        if (code < 0x80) {
            out += static_cast<char>(code);
        } else if (code < 0x800) {
            out += static_cast<char>(0xC0 | (code >> 6));
            out += static_cast<char>(0x80 | (code & 0x3F));
        } else if (code < 0x10000) {
            out += static_cast<char>(0xE0 | (code >> 12));
            out += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
            out += static_cast<char>(0x80 | (code & 0x3F));
        } else {
            out += static_cast<char>(0xF0 | (code >> 18));
            out += static_cast<char>(0x80 | ((code >> 12) & 0x3F));
            out += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
            out += static_cast<char>(0x80 | (code & 0x3F));
        }
    }
    unsigned hex4() {
        if (i_ + 4 > s_.size()) fail();
        unsigned code = static_cast<unsigned>(std::stoul(s_.substr(i_, 4), nullptr, 16));
        i_ += 4;
        return code;
    }
    std::string string() {
        if (!take("\"")) fail();
        std::string out;
        while (i_ < s_.size() && s_[i_] != '"') {
            char c = s_[i_++];
            if (c != '\\') {
                out += c;
                continue;
            }
            if (i_ >= s_.size()) fail();
            char e = s_[i_++];
            switch (e) {
            case 'n': out += '\n'; break;
            case 't': out += '\t'; break;
            case 'r': out += '\r'; break;
            case 'b': out += '\b'; break;
            case 'f': out += '\f'; break;
            case 'u': {
                unsigned code = hex4();
                if (code >= 0xD800 && code < 0xDC00 && take("\\u")) {
                    code = 0x10000 + ((code - 0xD800) << 10) + (hex4() - 0xDC00);
                }
                utf8(code, out);
                break;
            }
            default: out += e;
            }
        }
        if (!take("\"")) fail();
        return out;
    }

    const std::string& s_;
    std::size_t i_ = 0;
};

Json parse(const std::string& text) { return Parser(text).parse(); }

std::string dump(const Json& v) {
    switch (v.type) {
    case Json::Null: return "null";
    case Json::Bool: return v.boolean ? "true" : "false";
    case Json::Number: {
        std::ostringstream out;
        out.precision(17);
        out << v.number;
        return out.str();
    }
    case Json::String: return apeiron::json_string(v.text);
    case Json::Array: {
        std::string out = "[";
        for (std::size_t i = 0; i < v.items.size(); ++i) out += (i ? "," : "") + dump(v.items[i]);
        return out + "]";
    }
    case Json::Object: {
        std::string out = "{";
        bool first = true;
        for (const auto& [key, value] : v.fields) {
            out += (first ? "" : ",") + apeiron::json_string(key) + ":" + dump(value);
            first = false;
        }
        return out + "}";
    }
    }
    return "";
}

/// Objects by key, lists element by element with the same length, other
/// values by equality, as in the other runners.
bool holds(const Json& actual, const Json& expected) {
    if (actual.type != expected.type) return false;
    switch (expected.type) {
    case Json::Object:
        return std::all_of(expected.fields.begin(), expected.fields.end(), [&](const auto& field) {
            const Json* got = actual.get(field.first);
            return got != nullptr && holds(*got, field.second);
        });
    case Json::Array:
        if (actual.items.size() != expected.items.size()) return false;
        for (std::size_t i = 0; i < expected.items.size(); ++i) {
            if (!holds(actual.items[i], expected.items[i])) return false;
        }
        return true;
    case Json::String: return actual.text == expected.text;
    case Json::Number: return actual.number == expected.number;
    case Json::Bool: return actual.boolean == expected.boolean;
    case Json::Null: return true;
    }
    return false;
}

void expand(Json& v, const std::string& tmp) {
    if (v.type == Json::String) {
        for (std::size_t at = 0; (at = v.text.find("$TMP", at)) != std::string::npos; at += tmp.size()) {
            v.text.replace(at, 4, tmp);
        }
    }
    for (auto& item : v.items) expand(item, tmp);
    for (auto& field : v.fields) expand(field.second, tmp);
}

std::string read_all(const fs::path& path) {
    std::ifstream in(path, std::ios::binary);
    if (!in) throw std::runtime_error("cannot read " + path.string());
    return {std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>()};
}

void write_all(const fs::path& path, std::string_view data) {
    fs::create_directories(path.parent_path());
    std::ofstream out(path, std::ios::binary);
    out.write(data.data(), static_cast<std::streamsize>(data.size()));
}

/// A new folder that removes itself.
class Temp {
public:
    Temp() {
        std::random_device random;
        path_ = fs::temp_directory_path() / ("apeiron-cpp-" + std::to_string(random()) + std::to_string(random()));
        fs::create_directories(path_);
    }
    Temp(const Temp&) = delete;
    Temp& operator=(const Temp&) = delete;
    ~Temp() {
        std::error_code ignored;
        fs::remove_all(path_, ignored);
    }
    const fs::path& path() const { return path_; }
    std::string text() const { return path_.u8string(); }

private:
    fs::path path_;
};

int failures = 0;

void problem(const std::string& what) {
    std::cerr << "FAILED: " << what << "\n";
    ++failures;
}

#define CHECK(condition)                                                              \
    do {                                                                              \
        if (!(condition)) problem(std::string(__func__) + ": " #condition);           \
    } while (false)

/// Whether `f` throws exactly the error class E, with that kind.
template <class E, class F>
bool throws(const std::string& kind, F&& f) {
    try {
        f();
    } catch (const E& e) {
        return typeid(e) == typeid(E) && e.kind() == kind;
    } catch (const std::exception&) {
        return false;
    }
    return false;
}

void run_scenario(const fs::path& file) {
    Json scenario = parse(read_all(file));
    Temp tmp;
    std::optional<apeiron::Repository> repo;
    int number = 0;
    for (Json step : scenario.get("steps")->items) {
        ++number;
        expand(step, tmp.text());
        std::string at = file.filename().string() + " step " + std::to_string(number) + ": " + dump(step);
        const Json* request_value = step.get("request");
        std::string request = request_value != nullptr ? dump(*request_value) : "{}";
        const Json* password = step.get("password");
        const Json* expected_error = step.get("error");
        const Json* expected = step.get("expect");
        if (const Json* path = step.get("write")) {
            write_all(tmp.path() / fs::u8path(path->text), step.get("text")->text);
            continue;
        }
        if (const Json* path = step.get("remove")) {
            fs::remove(tmp.path() / fs::u8path(path->text));
            continue;
        }
        if (const Json* path = step.get("read")) {
            if (read_all(tmp.path() / fs::u8path(path->text)) != step.get("text")->text) problem(at);
            continue;
        }
        std::string result;
        try {
            if (const Json* path = step.get("init")) {
                std::string where = (tmp.path() / fs::u8path(path->text)).u8string();
                repo.reset();
                if (password != nullptr) {
                    auto [made, key] = apeiron::Repository::init_encrypted(where, password->text);
                    if (key.empty()) problem(at + ": no recovery key");
                    repo.emplace(std::move(made));
                } else {
                    repo.emplace(apeiron::Repository::init(where));
                }
                continue;
            }
            if (const Json* path = step.get("open")) {
                std::string where = (tmp.path() / fs::u8path(path->text)).u8string();
                auto opened = password != nullptr ? apeiron::Repository::open(where, password->text)
                                                  : apeiron::Repository::open(where);
                repo.reset();
                repo.emplace(std::move(opened));
                result = "null";
            } else if (const Json* op = step.get("call")) {
                result = repo->call(op->text, request);
            } else if (const Json* started = step.get("start")) {
                result = repo->start(started->text, request).result();
            } else {
                problem(at + ": unknown step");
                continue;
            }
        } catch (const apeiron::Error& e) {
            if (expected_error == nullptr || expected_error->text != e.kind()) {
                problem(at + ": " + e.kind() + ": " + e.what());
            }
            continue;
        }
        if (expected_error != nullptr) {
            problem(at + ": expected an error, got " + result);
        } else if (expected != nullptr && !holds(parse(result), *expected)) {
            problem(at + "\ngot " + result);
        }
    }
}

std::string noise(std::size_t length) {
    std::mt19937 random(12345);
    std::string out(length, '\0');
    std::generate(out.begin(), out.end(), [&] { return static_cast<char>(random()); });
    return out;
}

std::string read_whole(apeiron::FileReader& reader) {
    std::string out;
    std::vector<char> buffer(4099);
    while (std::size_t got = reader.read(buffer.data(), buffer.size())) {
        out.append(buffer.data(), got);
    }
    return out;
}

void errors_have_classes_and_details() {
    Temp tmp;
    auto repo = apeiron::Repository::init(tmp.text() + "/repo");
    CHECK(throws<apeiron::NotFoundError>("not_found", [&] { repo.call("resolve", R"({"spec":"main"})"); }));
    CHECK(throws<apeiron::UnsupportedError>("unsupported", [&] { repo.call("nonsense"); }));
    CHECK(throws<apeiron::InvalidInputError>("invalid_input", [&] { repo.call("resolve", "not json"); }));
    CHECK(throws<apeiron::InvalidInputError>("invalid_input", [&] { apeiron::Repository::open(tmp.text() + "/none"); }));
    try {
        repo.call("resolve", R"({"spec":"a \"quoted\" name"})");
        problem("no error");
    } catch (const apeiron::Error& e) {
        CHECK(e.code() != 0);
        // The library's message, its escapes undone once.
        CHECK(std::string(e.what()) == R"("a \"quoted\" name" is neither a ref name nor a snapshot id)");
    }
    CHECK(apeiron::json_string("a\"b\\c\n") == R"("a\"b\\c\u000a")");
    CHECK(parse(apeiron::json_string("şöğ\t\"\\")).text == "şöğ\t\"\\");
}

void jobs_wait_watch_and_cancel() {
    Temp tmp;
    write_all(tmp.path() / "src/a.bin", noise(20 << 20));
    auto repo = apeiron::Repository::init(tmp.text() + "/repo");
    std::string request = R"({"source":)" + apeiron::json_string(tmp.text() + "/src") + R"(,"ref":"main"})";

    int calls = 0;
    auto job = repo.start("backup", request);
    std::string done = job.watch([&](const std::string& progress) {
        CHECK(parse(progress).type == Json::Object);
        ++calls;
    }, std::chrono::milliseconds(1));
    CHECK(parse(done).get("files")->number == 1);

    // A result is given once.
    CHECK(throws<apeiron::InternalError>("internal", [&] { job.result(); }));

    // A job that is cancelled at once may still have finished; both are fine.
    write_all(tmp.path() / "src/b.bin", noise(30 << 20));
    auto stopped = repo.start("backup", request);
    stopped.cancel();
    try {
        stopped.result();
    } catch (const apeiron::CancelledError&) {
    }
    // Freed while it runs: the handle cancels it and waits.
    {
        auto left = repo.start("verify");
    }
    auto report = repo.start("verify").result();
    CHECK(parse(report).get("damaged_files")->items.empty());
}

void handles_move_and_free_themselves() {
    Temp tmp;
    static_assert(!std::is_copy_constructible_v<apeiron::Repository>);
    static_assert(std::is_nothrow_move_constructible_v<apeiron::Repository>);
    static_assert(!std::is_copy_assignable_v<apeiron::Job>);
    static_assert(std::is_nothrow_move_assignable_v<apeiron::FileWriter>);
    auto first = apeiron::Repository::init(tmp.text() + "/one");
    auto second = apeiron::Repository::init(tmp.text() + "/two");
    second = std::move(first);
    CHECK(second.call("refs").find('[') != std::string::npos);
    apeiron::Repository moved = std::move(second);
    // An empty handle, as a moved-from one is, fails instead of crashing.
    CHECK(throws<apeiron::InvalidInputError>("invalid_input", [&] { apeiron::Job(nullptr).progress(); }));
    CHECK(moved.call("refs").find('[') != std::string::npos);
    auto [encrypted, key] = apeiron::Repository::init_encrypted(tmp.text() + "/secret", "correct horse");
    CHECK(!key.empty());
    CHECK(parse(encrypted.call("info")).get("encrypted")->boolean);
    CHECK(throws<apeiron::PasswordError>("key", [&] { apeiron::Repository::open(tmp.text() + "/secret", "wrong"); }));
    CHECK(parse(apeiron::Repository::open(tmp.text() + "/secret", key).call("info")).get("encrypted")->boolean);
}

void a_file_goes_in_and_comes_out_as_a_stream() {
    Temp tmp;
    auto repo = apeiron::Repository::init(tmp.text() + "/repo");
    std::string data = noise(3 << 20);
    data[10] = '\0';

    auto writer = repo.add_file(R"({"ref":"inbox","folder":"in","name":"şğ.bin"})");
    writer.write(std::string_view(data).substr(0, 1000));
    writer.write(data.data() + 1000, data.size() - 1000);
    writer.write("");
    Json report = parse(writer.finish());
    CHECK(report.get("path")->text == "in/şğ.bin");
    CHECK(report.get("size")->number == static_cast<double>(data.size()));
    CHECK(throws<apeiron::InvalidInputError>("invalid_input", [&] { writer.finish(); }));

    auto reader = repo.open_file("inbox", "in/şğ.bin");
    CHECK(read_whole(reader) == data);
    char byte;
    CHECK(reader.read(&byte, 1) == 0);
    CHECK(throws<apeiron::NotFoundError>("not_found", [&] { repo.open_file("inbox", "missing"); }));
    CHECK(throws<apeiron::InvalidInputError>("invalid_input", [&] { reader.read(&byte, 0); }));

    // Leaving the scope early, by an exception, stores nothing.
    std::string before = repo.call("log", R"({"start":"inbox"})");
    try {
        auto unfinished = repo.add_file(R"({"ref":"inbox","name":"half.bin"})");
        unfinished.write(data);
        throw std::runtime_error("stop");
    } catch (const std::runtime_error&) {
    }
    CHECK(repo.call("log", R"({"start":"inbox"})") == before);
    CHECK(throws<apeiron::InvalidInputError>("invalid_input", [&] { repo.add_file(R"({"ref":"inbox","name":".."})"); }));
    CHECK(parse(repo.start("verify").result()).get("damaged_objects")->items.empty());
}

void no_repository_needed() {
    Temp tmp;
    std::string request = R"({"dir":)" + apeiron::json_string(tmp.text() + "/plugins") + "}";
    CHECK(parse(apeiron::call("plugins", request)).type == Json::Array);
    write_all(tmp.path() / "src/a.txt", "one");
    {
        auto repo = apeiron::Repository::init(tmp.text() + "/repo");
        repo.start("backup", R"({"source":)" + apeiron::json_string(tmp.text() + "/src") + R"(,"ref":"main"})").result();
    }
    std::string clone = R"({"url":)" + apeiron::json_string(tmp.text() + "/repo") + R"(,"path":)" +
                        apeiron::json_string(tmp.text() + "/copy") + "}";
    apeiron::clone(clone).result();
    CHECK(parse(apeiron::Repository::open(tmp.text() + "/copy").call("refs")).items.size() == 1);
}

} // namespace

int main(int argc, char** argv) {
    if (argc != 2) {
        std::cerr << "usage: run <folder of the scenarios>\n";
        return 2;
    }
    try {
        int scenarios = 0;
        std::vector<fs::path> files;
        for (const auto& entry : fs::directory_iterator(argv[1])) {
            if (entry.path().extension() == ".json") files.push_back(entry.path());
        }
        std::sort(files.begin(), files.end());
        for (const auto& file : files) {
            run_scenario(file);
            ++scenarios;
        }
        if (scenarios < 4) problem("too few scenarios: " + std::to_string(scenarios));
        errors_have_classes_and_details();
        jobs_wait_watch_and_cancel();
        handles_move_and_free_themselves();
        a_file_goes_in_and_comes_out_as_a_stream();
        no_repository_needed();
        std::cout << scenarios << " scenarios; " << failures << " failures\n";
    } catch (const std::exception& e) {
        std::cerr << "FAILED: " << e.what() << "\n";
        return 1;
    }
    return failures == 0 ? 0 : 1;
}
