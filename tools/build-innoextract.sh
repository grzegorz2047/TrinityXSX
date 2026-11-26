#!/usr/bin/env bash
# Build self-contained static-deps trinity_innoextract.dll (MSYS2 UCRT64) from dscharrer/innoextract.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=../pins.env
. "$root/pins.env"

src="$root/innoextract-src"
build="$src/build-dll"
out="$root/out/innoextract"

rm -rf "$src" "$out"
git init -q "$src"
git -C "$src" fetch -q --depth 1 "$INNOEXTRACT_REPO" "$INNOEXTRACT_REV"
git -C "$src" checkout -q FETCH_HEAD

# 1. Hook progress::update() in src/util/console.cpp to report byte progress without touching stdout
python3 - "$src" "$root/tools/innoextract/trinity_inno_api.cpp" <<'PY'
import pathlib, shutil, sys

src = pathlib.Path(sys.argv[1])
api_src = pathlib.Path(sys.argv[2])
shutil.copy2(api_src, src / "src" / "cli" / "trinity_inno_api.cpp")

console_cpp = src / "src" / "util" / "console.cpp"
txt = console_cpp.read_text(encoding="utf-8")
inc_anchor = '#include "util/console.hpp"\n'
assert inc_anchor in txt, "console.hpp include not found in console.cpp"
hook_decl = (
    '#include "util/console.hpp"\n'
    '#include <cstdint>\n'
    'extern "C" void trinity_inno_notify_progress(uint64_t done_bytes, uint64_t total_bytes);\n'
)
if "trinity_inno_notify_progress" not in txt:
    txt = txt.replace(inc_anchor, hook_decl, 1)
txt = txt.replace("isatty(", "util::isatty(")
needle = "\tvalue += delta;\n"
assert needle in txt, "progress::update value += delta not found"
replacement = (
    "\tvalue += delta;\n"
    "\ttrinity_inno_notify_progress(value, max);\n"
    "\treturn true;\n"
)
txt = txt.replace(needle, replacement, 1)
console_cpp.write_text(txt, encoding="utf-8")

# Avoid GCC 16 / UCRT64 <io.h> conflict with `using util::isatty;`
win_hpp = src / "src" / "util" / "windows.hpp"
wh = win_hpp.read_text(encoding="utf-8")
assert "using util::isatty;" in wh, "using util::isatty; not found in windows.hpp"
wh = wh.replace("using util::isatty;", "// using util::isatty; // conflicts with UCRT64 <io.h>")
win_hpp.write_text(wh, encoding="utf-8")

# Replace CLI main() wrapper in windows.cpp with DLL-safe UTF-8 Boost.Filesystem locale initializer
win_cpp = src / "src" / "util" / "windows.cpp"
wc = win_cpp.read_text(encoding="utf-8")
main_anchor = "// We really want main here, not utf8_main."
assert main_anchor in wc, "main() wrapper not found in windows.cpp"
wc = wc[:wc.index(main_anchor)] + (
    'extern "C" void trinity_inno_init_utf8_locale() {\n'
    '\tstatic bool inited = false;\n'
    '\tif (!inited) {\n'
    '\t\tinited = true;\n'
    '\t\tboost::filesystem::path::imbue(std::locale(std::locale(), new utf8_codecvt()));\n'
    '\t}\n'
    '}\n'
)
win_cpp.write_text(wc, encoding="utf-8")

# Fix Boost 1.88+ indirect_streambuf::close() dropping the final <4100-byte put-area buffer
# on seekable file_descriptor streams (truncating every extracted file to floor(size/4100)*4100):
# 1) Replace file_output's stream_ in src/cli/extract.cpp with direct Win32 CreateFileW/WriteFile/
#    ReadFile/SetFilePointerEx/CloseHandle + explicit size check on close (also ~20x fewer syscalls).
# 2) Ensure util::path_fstream flushes before close() in src/util/fstream.hpp.
fstream_hpp = src / "src" / "util" / "fstream.hpp"
fh = fstream_hpp.read_text(encoding="utf-8")
old_fh_methods = """\tvoid open(const path & p) {
\t\tbase::close();
\t\tbase::open(p);
\t}

\tvoid open(const path & p, std::ios_base::openmode mode) {
\t\tbase::close();
\t\tbase::open(p, mode);
\t\tfix_open_mode(mode);
\t}
\t
\tbool is_open() {
\t\treturn device().is_open(); // return the real open state, not base::is_open()
\t}
\t
\tvirtual ~path_fstream() { }"""
new_fh_methods = """\tvoid flush_before_close();

\tvoid close() {
\t\tflush_before_close();
\t\tbase::close();
\t}

\tvoid open(const path & p) {
\t\tclose();
\t\tbase::open(p);
\t}

\tvoid open(const path & p, std::ios_base::openmode mode) {
\t\tclose();
\t\tbase::open(p, mode);
\t\tfix_open_mode(mode);
\t}
\t
\tbool is_open() {
\t\treturn device().is_open(); // return the real open state, not base::is_open()
\t}
\t
\tvirtual ~path_fstream() {
\t\ttry {
\t\t\tclose();
\t\t} catch(...) { }
\t}"""
assert old_fh_methods in fh, "path_fstream methods not found in util/fstream.hpp"
fh = fh.replace(old_fh_methods, new_fh_methods, 1)
old_fh_typedefs = "typedef path_fstream<boost::iostreams::file_descriptor_source> ifstream;"
new_fh_typedefs = """template <>
inline void path_fstream<boost::iostreams::file_descriptor_source>::flush_before_close() { }

template <>
inline void path_fstream<boost::iostreams::file_descriptor_sink>::flush_before_close() {
\tif(is_open()) {
\t\tthis->flush();
\t}
}

template <>
inline void path_fstream<boost::iostreams::file_descriptor>::flush_before_close() {
\tif(is_open()) {
\t\tthis->flush();
\t}
}

typedef path_fstream<boost::iostreams::file_descriptor_source> ifstream;"""
assert old_fh_typedefs in fh, "path_fstream typedefs not found in util/fstream.hpp"
fh = fh.replace(old_fh_typedefs, new_fh_typedefs, 1)
fstream_hpp.write_text(fh, encoding="utf-8")

extract_cpp = src / "src" / "cli" / "extract.cpp"
ec = extract_cpp.read_text(encoding="utf-8")
ec_inc = '#include "cli/extract.hpp"\n'
assert ec_inc in ec, "extract.hpp include not found in extract.cpp"
ec = ec.replace(
    ec_inc,
    '#ifdef _WIN32\n'
    '#ifndef WIN32_LEAN_AND_MEAN\n#define WIN32_LEAN_AND_MEAN\n#endif\n'
    '#ifndef NOMINMAX\n#define NOMINMAX\n#endif\n'
    '#include <windows.h>\n'
    '#endif\n'
    + ec_inc,
    1,
)
start_marker = "class file_output : private boost::noncopyable {"
end_marker = "\nclass path_filter {"
s_idx = ec.index(start_marker)
e_idx = ec.index(end_marker, s_idx)
direct_file_output = """class file_output : private boost::noncopyable {
\t
\tfs::path path_;
\tconst processed_file * file_;
\tHANDLE handle_;
\t
\tcrypto::hasher checksum_;
\tboost::uint64_t checksum_position_;
\t
\tboost::uint64_t position_;
\tboost::uint64_t total_written_;
\t
\tbool write_;
\t
public:
\t
\texplicit file_output(const fs::path & dir, const processed_file * f, bool write)
\t\t: path_(dir / f->path())
\t\t, file_(f)
\t\t, handle_(INVALID_HANDLE_VALUE)
\t\t, checksum_(f->entry().checksum.type)
\t\t, checksum_position_(f->entry().checksum.type == crypto::None ? boost::uint64_t(-1) : 0)
\t\t, position_(0)
\t\t, total_written_(0)
\t\t, write_(write)
\t{
\t\tif(write_) {
\t\t\tstd::wstring wpath = path_.wstring();
\t\t\tif(wpath.size() >= 248 && wpath.compare(0, 4, L"\\\\\\\\?\\\\") != 0 && wpath.size() >= 2 && wpath[1] == L':') {
\t\t\t\tstd::replace(wpath.begin(), wpath.end(), L'/', L'\\\\');
\t\t\t\twpath = L"\\\\\\\\?\\\\" + wpath;
\t\t\t}
\t\t\tDWORD access = GENERIC_WRITE | (file_->is_multipart() ? GENERIC_READ : 0);
\t\t\thandle_ = CreateFileW(
\t\t\t\twpath.c_str(),
\t\t\t\taccess,
\t\t\t\tFILE_SHARE_READ,
\t\t\t\tNULL,
\t\t\t\tCREATE_ALWAYS,
\t\t\t\tFILE_ATTRIBUTE_NORMAL,
\t\t\t\tNULL);
\t\t\tif(handle_ == INVALID_HANDLE_VALUE) {
\t\t\t\tthrow std::runtime_error("Could not open output file \\"" + path_.string() + '"');
\t\t\t}
\t\t}
\t}
\t
\t~file_output() {
\t\tif(write_ && handle_ != INVALID_HANDLE_VALUE) {
\t\t\tCloseHandle(handle_);
\t\t\thandle_ = INVALID_HANDLE_VALUE;
\t\t}
\t}
\t
\tbool write(const char * data, size_t n) {
\t\tif(write_) {
\t\t\tif(handle_ == INVALID_HANDLE_VALUE) {
\t\t\t\treturn false;
\t\t\t}
\t\t\tconst char * ptr = data;
\t\t\tsize_t rem = n;
\t\t\twhile(rem > 0) {
\t\t\t\tDWORD chunk = rem > 0x40000000u ? 0x40000000u : DWORD(rem);
\t\t\t\tDWORD written = 0;
\t\t\t\tif(!WriteFile(handle_, ptr, chunk, &written, NULL) || written == 0) {
\t\t\t\t\treturn false;
\t\t\t\t}
\t\t\t\tptr += written;
\t\t\t\trem -= written;
\t\t\t}
\t\t}
\t\tif(checksum_position_ == position_) {
\t\t\tchecksum_.update(data, n);
\t\t\tchecksum_position_ += n;
\t\t}
\t\tposition_ += n;
\t\ttotal_written_ += n;
\t\treturn true;
\t}
\t
\tvoid seek(boost::uint64_t new_position) {
\t\tif(new_position == position_) {
\t\t\treturn;
\t\t}
\t\tdebug("seeking output from " << print_hex(position_) << " to " << print_hex(new_position));
\t\tif(!write_) {
\t\t\tposition_ = new_position;
\t\t\treturn;
\t\t}
\t\tLARGE_INTEGER li;
\t\tli.QuadPart = LONGLONG(new_position);
\t\tSetFilePointerEx(handle_, li, NULL, FILE_BEGIN);
\t\tposition_ = new_position;
\t}
\t
\tvoid close() {
\t\tif(write_ && handle_ != INVALID_HANDLE_VALUE) {
\t\t\tLARGE_INTEGER disk_sz;
\t\t\tbool has_sz = GetFileSizeEx(handle_, &disk_sz) != 0;
\t\t\tBOOL ok = CloseHandle(handle_);
\t\t\thandle_ = INVALID_HANDLE_VALUE;
\t\t\tif(!ok) {
\t\t\t\tthrow std::runtime_error("Error closing output file \\"" + path_.string() + '"');
\t\t\t}
\t\t\tif(has_sz && !file_->is_multipart() && boost::uint64_t(disk_sz.QuadPart) < total_written_) {
\t\t\t\tthrow std::runtime_error("Truncated output file \\"" + path_.string() + '"');
\t\t\t}
\t\t}
\t}
\t
\tconst fs::path & path() const { return path_; }
\tconst processed_file * file() const { return file_; }
\t
\tbool is_complete() const {
\t\treturn total_written_ == file_->entry().size;
\t}
\t
\tbool has_checksum() const {
\t\treturn checksum_position_ == file_->entry().size;
\t}
\t
\tbool calculate_checksum() {
\t\tif(has_checksum()) {
\t\t\treturn true;
\t\t}
\t\tif(!write_ || handle_ == INVALID_HANDLE_VALUE) {
\t\t\treturn false;
\t\t}
\t\tdebug("calculating output checksum for " << path_);
\t\tLARGE_INTEGER li;
\t\tli.QuadPart = LONGLONG(checksum_position_);
\t\tSetFilePointerEx(handle_, li, NULL, FILE_BEGIN);
\t\tfor(;;) {
\t\t\tchar buffer[8192];
\t\t\tDWORD n = 0;
\t\t\tif(!ReadFile(handle_, buffer, sizeof(buffer), &n, NULL) || n == 0) {
\t\t\t\tbreak;
\t\t\t}
\t\t\tchecksum_.update(buffer, size_t(n));
\t\t\tchecksum_position_ += boost::uint64_t(n);
\t\t}
\t\tposition_ = checksum_position_;
\t\tif(!has_checksum()) {
\t\t\tlog_warning << "Could not read back " << path_ << " to calculate output checksum for multi-part file";
\t\t\treturn false;
\t\t}
\t\treturn true;
\t}
\t
\tcrypto::checksum checksum() {
\t\treturn checksum_.finalize();
\t}
\t
};
"""
ec = ec[:s_idx] + direct_file_output + ec[e_idx:]
old_close_block = """\t\t\t\t// Adjust file timestamps
\t\t\t\tif(o.extract && o.preserve_file_times) {
\t\t\t\t\toutput->close();
\t\t\t\t\tif(!util::set_file_time(output->path(), filetime, data.timestamp_nsec)) {
\t\t\t\t\t\tlog_warning << "Error setting timestamp on file " << output->path();
\t\t\t\t\t}
\t\t\t\t}"""
new_close_block = """\t\t\t\t// Explicitly close and verify every extracted file before touching timestamps
\t\t\t\tif(o.extract) {
\t\t\t\t\toutput->close();
\t\t\t\t\tif(o.preserve_file_times && !util::set_file_time(output->path(), filetime, data.timestamp_nsec)) {
\t\t\t\t\t\tlog_warning << "Error setting timestamp on file " << output->path();
\t\t\t\t\t}
\t\t\t\t}"""
assert old_close_block in ec, "preserve_file_times close block not found in extract.cpp"
ec = ec.replace(old_close_block, new_close_block, 1)
extract_cpp.write_text(ec, encoding="utf-8")

cmake = src / "CMakeLists.txt"
ct = cmake.read_text(encoding="utf-8")
ct = ct.replace("src/cli/main.cpp", "src/cli/trinity_inno_api.cpp")
old_target = "add_executable(innoextract ${INNOEXTRACT_SOURCES})\ntarget_link_libraries(innoextract ${LIBRARIES})"
new_target = (
    "add_library(innoextract SHARED ${INNOEXTRACT_SOURCES})\n"
    "set_target_properties(innoextract PROPERTIES PREFIX \"\" OUTPUT_NAME \"trinity_innoextract\")\n"
    "target_link_options(innoextract PRIVATE -static-libgcc -static-libstdc++)\n"
    "target_link_libraries(innoextract ${LIBRARIES} -Wl,-Bstatic -lstdc++ -lwinpthread -Wl,-Bdynamic)"
)
assert old_target in ct, "innoextract CMake target not found"
ct = ct.replace(old_target, new_target, 1)
cmake.write_text(ct, encoding="utf-8")
print("Patched innoextract CMakeLists.txt & console.cpp for static trinity_innoextract.dll")
PY

cmake -S "$src" -B "$build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_FIND_LIBRARY_SUFFIXES=".a" \
  -DUSE_LTO=OFF \
  -DUSE_STATIC_LIBS=ON \
  -DLZMA_USE_STATIC_LIBS=ON \
  -DZLIB_USE_STATIC_LIBS=ON \
  -DBZip2_USE_STATIC_LIBS=ON \
  -DBoost_USE_STATIC_LIBS=ON \
  -Diconv_USE_STATIC_LIBS=ON \
  -DWITH_CONV=win32 \
  -DBUILD_TESTS=OFF \
  -DSET_WARNING_FLAGS=OFF

# Boost's MSYS2 CMake targets hardcode .dll.a for transitive compression libs (bz2/lzma/zstd).
# Rewrite them to their sibling static .a archives so trinity_innoextract.dll has zero MSYS2 DLL deps.
python3 - "$build/build.ninja" <<'PY'
import pathlib, re, sys
ninja_path = pathlib.Path(sys.argv[1])
txt = ninja_path.read_text(encoding="utf-8")
replaced = set()
def repl(m):
    full = m.group(0)
    static_p = full[:-6] + ".a"
    if pathlib.Path(static_p).is_file():
        replaced.add((full, static_p))
        return static_p
    print(f"WARNING: static archive missing for {full}: {static_p}", file=sys.stderr)
    return full
txt = re.sub(r'[^\s"]+lib(?:bz2|lzma|zstd|z|iconv|winpthread)\.dll\.a', repl, txt)
ninja_path.write_text(txt, encoding="utf-8")
for old, new in sorted(replaced):
    print(f"Static link override: {old} -> {new}")
PY

cmake --build "$build" --target innoextract -j "$(nproc)"

mkdir -p "$out"
cp "$build/trinity_innoextract.dll" "$out/"
cp "$src/LICENSE" "$out/innoextract-zlib.txt"
strip --strip-unneeded "$out/trinity_innoextract.dll" || true
ls -lh "$out/"
objdump -p "$out/trinity_innoextract.dll" | grep "DLL Name:" || true
# Fail fast if any non-Windows MSYS2 DLL leaked into imports
if objdump -p "$out/trinity_innoextract.dll" | grep -Ei "DLL Name:.*(libgcc|libstdc\+\+|libwinpthread|libboost|liblzma|libbz2|libz|libzstd|libiconv)"; then
  echo "ERROR: trinity_innoextract.dll has non-static MSYS2 DLL dependency!" >&2
  exit 1
fi
