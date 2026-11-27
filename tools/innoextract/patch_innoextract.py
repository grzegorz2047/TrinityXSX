#!/usr/bin/env python3
# SPDX-License-Identifier: Zlib
"""Patch upstream dscharrer/innoextract into a high-throughput static trinity_innoextract.dll.

Key patches applied:
1. Progress callback hook in src/util/console.cpp (atomic byte progress without stdout).
2. DLL-safe UTF-8 Boost.Filesystem locale initializer in src/util/windows.cpp + GCC 16 <io.h> fix.
3. Flush-before-close fix for Boost 1.88+ path_fstream in src/util/fstream.hpp.
4. Direct Win32 4 MiB write-buffered file_output + skip unused MD5/SHA-1 when !o.test.
5. Persistent multi-threaded (up to 6 cores) parallel zlib worker pool in src/cli/extract.cpp
   for independent plaintext single-file zlib chunks (GOG Galaxy / Inno zlib).
6. Direct Win32 4 MiB read-ahead buffered slice_reader in src/stream/slice.{hpp,cpp}.
7. Larger 256 KiB filter buffers in src/stream/{chunk,file}.cpp + 1 MiB serial copy buffer.
8. Shared DLL target (trinity_innoextract.dll) with static libgcc/libstdc++/libwinpthread.
"""

import pathlib
import shutil
import sys


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit(f"usage: {sys.argv[0]} <innoextract-src-dir> <tools-innoextract-dir>")

    src = pathlib.Path(sys.argv[1])
    overlay = pathlib.Path(sys.argv[2])

    shutil.copy2(overlay / "trinity_inno_api.cpp", src / "src" / "cli" / "trinity_inno_api.cpp")
    shutil.copy2(overlay / "trinity_fast_io.hpp", src / "src" / "util" / "trinity_fast_io.hpp")

    # 1. Hook progress::update() in src/util/console.cpp
    console_cpp = src / "src" / "util" / "console.cpp"
    txt = console_cpp.read_text(encoding="utf-8")
    inc_anchor = '#include "util/console.hpp"\n'
    assert inc_anchor in txt, "console.hpp include not found in console.cpp"
    hook_decl = (
        '#include "util/console.hpp"\n'
        "#include <cstdint>\n"
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
    console_cpp.write_text(txt, encoding="utf-8", newline="\n")

    # 2. Avoid GCC 16 / UCRT64 <io.h> conflict with `using util::isatty;`
    win_hpp = src / "src" / "util" / "windows.hpp"
    wh = win_hpp.read_text(encoding="utf-8")
    assert "using util::isatty;" in wh, "using util::isatty; not found in windows.hpp"
    wh = wh.replace("using util::isatty;", "// using util::isatty; // conflicts with UCRT64 <io.h>")
    win_hpp.write_text(wh, encoding="utf-8", newline="\n")

    # 3. Replace CLI main() wrapper in windows.cpp with DLL-safe UTF-8 locale initializer
    win_cpp = src / "src" / "util" / "windows.cpp"
    wc = win_cpp.read_text(encoding="utf-8")
    main_anchor = "// We really want main here, not utf8_main."
    assert main_anchor in wc, "main() wrapper not found in windows.cpp"
    wc = wc[: wc.index(main_anchor)] + (
        'extern "C" void trinity_inno_init_utf8_locale() {\n'
        "\tstatic bool inited = false;\n"
        "\tif (!inited) {\n"
        "\t\tinited = true;\n"
        "\t\tboost::filesystem::path::imbue(std::locale(std::locale(), new utf8_codecvt()));\n"
        "\t}\n"
        "}\n"
    )
    win_cpp.write_text(wc, encoding="utf-8", newline="\n")

    # 4. Flush before close in util::path_fstream (Boost 1.88+ indirect_streambuf fix)
    fstream_hpp = src / "src" / "util" / "fstream.hpp"
    fh = fstream_hpp.read_text(encoding="utf-8")
    old_fh_methods = (
        "\tvoid open(const path & p) {\n"
        "\t\tbase::close();\n"
        "\t\tbase::open(p);\n"
        "\t}\n\n"
        "\tvoid open(const path & p, std::ios_base::openmode mode) {\n"
        "\t\tbase::close();\n"
        "\t\tbase::open(p, mode);\n"
        "\t\tfix_open_mode(mode);\n"
        "\t}\n"
        "\t\n"
        "\tbool is_open() {\n"
        "\t\treturn device().is_open(); // return the real open state, not base::is_open()\n"
        "\t}\n"
        "\t\n"
        "\tvirtual ~path_fstream() { }"
    )
    new_fh_methods = (
        "\tvoid flush_before_close();\n\n"
        "\tvoid close() {\n"
        "\t\tflush_before_close();\n"
        "\t\tbase::close();\n"
        "\t}\n\n"
        "\tvoid open(const path & p) {\n"
        "\t\tclose();\n"
        "\t\tbase::open(p);\n"
        "\t}\n\n"
        "\tvoid open(const path & p, std::ios_base::openmode mode) {\n"
        "\t\tclose();\n"
        "\t\tbase::open(p, mode);\n"
        "\t\tfix_open_mode(mode);\n"
        "\t}\n"
        "\t\n"
        "\tbool is_open() {\n"
        "\t\treturn device().is_open(); // return the real open state, not base::is_open()\n"
        "\t}\n"
        "\t\n"
        "\tvirtual ~path_fstream() {\n"
        "\t\ttry {\n"
        "\t\t\tclose();\n"
        "\t\t} catch(...) { }\n"
        "\t}"
    )
    assert old_fh_methods in fh, "path_fstream methods not found in util/fstream.hpp"
    fh = fh.replace(old_fh_methods, new_fh_methods, 1)
    old_fh_typedefs = "typedef path_fstream<boost::iostreams::file_descriptor_source> ifstream;"
    new_fh_typedefs = (
        "template <>\n"
        "inline void path_fstream<boost::iostreams::file_descriptor_source>::flush_before_close() { }\n\n"
        "template <>\n"
        "inline void path_fstream<boost::iostreams::file_descriptor_sink>::flush_before_close() {\n"
        "\tif(is_open()) {\n"
        "\t\tthis->flush();\n"
        "\t}\n"
        "}\n\n"
        "template <>\n"
        "inline void path_fstream<boost::iostreams::file_descriptor>::flush_before_close() {\n"
        "\tif(is_open()) {\n"
        "\t\tthis->flush();\n"
        "\t}\n"
        "}\n\n"
        "typedef path_fstream<boost::iostreams::file_descriptor_source> ifstream;"
    )
    assert old_fh_typedefs in fh, "path_fstream typedefs not found in util/fstream.hpp"
    fh = fh.replace(old_fh_typedefs, new_fh_typedefs, 1)
    fstream_hpp.write_text(fh, encoding="utf-8", newline="\n")

    # 5. Patch src/cli/extract.cpp (4 MiB buffered file_output, skip unused hashes when !o.test, parallel zlib pool)
    patch_extract_cpp(src / "src" / "cli" / "extract.cpp")

    # 6. Patch src/stream/slice.{hpp,cpp} (4 MiB buffered Win32 reader for external .bin slices)
    patch_slice(src / "src" / "stream" / "slice.hpp", src / "src" / "stream" / "slice.cpp")

    # 7. Increase Boost.Iostreams filter buffers in serial path (8 KiB -> 256 KiB)
    for rel_p in ("src/stream/chunk.cpp", "src/stream/file.cpp"):
        fp = src / rel_p
        ft = fp.read_text(encoding="utf-8")
        assert ", 8192)" in ft, f"8192 buffer not found in {rel_p}"
        ft = ft.replace(", 8192)", ", 262144)")
        fp.write_text(ft, encoding="utf-8", newline="\n")

    # 8. Patch CMakeLists.txt to build static-deps trinity_innoextract.dll
    cmake = src / "CMakeLists.txt"
    ct = cmake.read_text(encoding="utf-8")
    ct = ct.replace("src/cli/main.cpp", "src/cli/trinity_inno_api.cpp")
    old_target = "add_executable(innoextract ${INNOEXTRACT_SOURCES})\ntarget_link_libraries(innoextract ${LIBRARIES})"
    new_target = (
        "add_library(innoextract SHARED ${INNOEXTRACT_SOURCES})\n"
        'set_target_properties(innoextract PROPERTIES PREFIX "" OUTPUT_NAME "trinity_innoextract")\n'
        "target_link_options(innoextract PRIVATE -static-libgcc -static-libstdc++)\n"
        "target_link_libraries(innoextract ${LIBRARIES} -Wl,-Bstatic -lstdc++ -lwinpthread -Wl,-Bdynamic)"
    )
    assert old_target in ct, "innoextract CMake target not found"
    ct = ct.replace(old_target, new_target, 1)
    cmake.write_text(ct, encoding="utf-8", newline="\n")
    print("Patched innoextract cleanly for high-throughput parallel trinity_innoextract.dll")


def patch_extract_cpp(extract_cpp: pathlib.Path) -> None:
    ec = extract_cpp.read_text(encoding="utf-8")
    ec_inc = '#include "cli/extract.hpp"\n'
    assert ec_inc in ec, "extract.hpp include not found in extract.cpp"
    ec = ec.replace(ec_inc, '#include "util/trinity_fast_io.hpp"\n' + ec_inc, 1)

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
\tboost::uint64_t hw_position_;
\tboost::uint64_t total_written_;
\t
\tstd::vector<char> wbuf_;
\tsize_t wbuf_used_;
\t
\tbool write_;
\tbool verify_checksum_;
\t
\tbool write_raw(const char * ptr, size_t rem) {
\t\twhile(rem > 0) {
\t\t\tDWORD chunk = rem > 0x40000000u ? 0x40000000u : DWORD(rem);
\t\t\tDWORD written = 0;
\t\t\tif(!WriteFile(handle_, ptr, chunk, &written, NULL) || written == 0) {
\t\t\t\treturn false;
\t\t\t}
\t\t\tptr += written;
\t\t\trem -= written;
\t\t\thw_position_ += written;
\t\t}
\t\treturn true;
\t}
\t
\tbool flush_wbuf() {
\t\tif(wbuf_used_ == 0) {
\t\t\treturn true;
\t\t}
\t\tbool ok = write_raw(wbuf_.data(), wbuf_used_);
\t\twbuf_used_ = 0;
\t\treturn ok;
\t}
\t
public:
\t
\texplicit file_output(const fs::path & dir, const processed_file * f, bool write, bool verify_checksum = false)
\t\t: path_(dir / f->path())
\t\t, file_(f)
\t\t, handle_(INVALID_HANDLE_VALUE)
\t\t, checksum_(verify_checksum ? f->entry().checksum.type : crypto::None)
\t\t, checksum_position_((!verify_checksum || f->entry().checksum.type == crypto::None) ? boost::uint64_t(-1) : 0)
\t\t, position_(0)
\t\t, hw_position_(0)
\t\t, total_written_(0)
\t\t, wbuf_used_(0)
\t\t, write_(write)
\t\t, verify_checksum_(verify_checksum)
\t{
\t\tif(write_) {
\t\t\tstd::wstring wpath = trinity_fast::to_long_win_path(path_.wstring());
\t\t\tDWORD access = GENERIC_WRITE | ((verify_checksum_ && file_->is_multipart()) ? GENERIC_READ : 0);
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
\t\t\tflush_wbuf();
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
\t\t\tif(n >= trinity_fast::kFileWriteBufBytes) {
\t\t\t\tif(!flush_wbuf() || !write_raw(data, n)) {
\t\t\t\t\treturn false;
\t\t\t\t}
\t\t\t} else if(n > 0) {
\t\t\t\tif(wbuf_.empty()) {
\t\t\t\t\twbuf_.resize(trinity_fast::kFileWriteBufBytes);
\t\t\t\t}
\t\t\t\tif(wbuf_used_ + n > wbuf_.size()) {
\t\t\t\t\tif(!flush_wbuf()) {
\t\t\t\t\t\treturn false;
\t\t\t\t\t}
\t\t\t\t}
\t\t\t\tstd::memcpy(wbuf_.data() + wbuf_used_, data, n);
\t\t\t\twbuf_used_ += n;
\t\t\t}
\t\t}
\t\tif(verify_checksum_ && checksum_position_ == position_) {
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
\t\tif(!flush_wbuf()) {
\t\t\tthrow std::runtime_error("Error flushing output file \\"" + path_.string() + '"');
\t\t}
\t\tif(hw_position_ != new_position) {
\t\t\tLARGE_INTEGER li;
\t\t\tli.QuadPart = LONGLONG(new_position);
\t\t\tSetFilePointerEx(handle_, li, NULL, FILE_BEGIN);
\t\t\thw_position_ = new_position;
\t\t}
\t\tposition_ = new_position;
\t}
\t
\tvoid close() {
\t\tif(write_ && handle_ != INVALID_HANDLE_VALUE) {
\t\t\tbool flushed = flush_wbuf();
\t\t\tLARGE_INTEGER disk_sz;
\t\t\tbool has_sz = GetFileSizeEx(handle_, &disk_sz) != 0;
\t\t\tBOOL ok = CloseHandle(handle_);
\t\t\thandle_ = INVALID_HANDLE_VALUE;
\t\t\tstd::vector<char>().swap(wbuf_);
\t\t\tif(!flushed || !ok) {
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
\t\treturn verify_checksum_ && checksum_position_ == file_->entry().size;
\t}
\t
\tbool calculate_checksum() {
\t\tif(!verify_checksum_) {
\t\t\treturn false;
\t\t}
\t\tif(has_checksum()) {
\t\t\treturn true;
\t\t}
\t\tif(!write_ || handle_ == INVALID_HANDLE_VALUE) {
\t\t\treturn false;
\t\t}
\t\tif(!flush_wbuf()) {
\t\t\treturn false;
\t\t}
\t\tdebug("calculating output checksum for " << path_);
\t\tLARGE_INTEGER li;
\t\tli.QuadPart = LONGLONG(checksum_position_);
\t\tSetFilePointerEx(handle_, li, NULL, FILE_BEGIN);
\t\tstd::vector<char> rbuf(262144u);
\t\tfor(;;) {
\t\t\tDWORD n = 0;
\t\t\tif(!ReadFile(handle_, rbuf.data(), DWORD(rbuf.size()), &n, NULL) || n == 0) {
\t\t\t\tbreak;
\t\t\t}
\t\t\tchecksum_.update(rbuf.data(), size_t(n));
\t\t\tchecksum_position_ += boost::uint64_t(n);
\t\t}
\t\thw_position_ = checksum_position_;
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

    old_new_out = "\t\t\t\t\t\toutput = new file_output(o.output_dir, fileinfo, o.extract);"
    new_new_out = "\t\t\t\t\t\toutput = new file_output(o.output_dir, fileinfo, o.extract, o.test);"
    assert old_new_out in ec, "new file_output call not found in extract.cpp"
    ec = ec.replace(old_new_out, new_new_out, 1)

    old_fr = "\t\t\tfile_source = stream::file_reader::get(*chunk_source, file, &checksum);"
    new_fr = "\t\t\tfile_source = stream::file_reader::get(*chunk_source, file, o.test ? &checksum : NULL);"
    assert old_fr in ec, "file_reader::get call not found in extract.cpp"
    ec = ec.replace(old_fr, new_fr, 1)

    old_chk_verify = "\t\t\t// Verify checksums\n\t\t\tif(checksum != file.checksum) {"
    new_chk_verify = "\t\t\t// Verify checksums\n\t\t\tif(o.test && checksum != file.checksum) {"
    assert old_chk_verify in ec, "checksum verify block not found in extract.cpp"
    ec = ec.replace(old_chk_verify, new_chk_verify, 1)

    old_copy_buf = (
        "\t\t\tboost::uint64_t output_size = 0;\n"
        "\t\t\twhile(!file_source->eof()) {\n"
        "\t\t\t\tchar buffer[8192 * 10];\n"
        "\t\t\t\tstd::streamsize buffer_size = std::streamsize(boost::size(buffer));\n"
        "\t\t\t\tstd::streamsize n = file_source->read(buffer, buffer_size).gcount();\n"
        "\t\t\t\tif(n > 0) {\n"
        "\t\t\t\t\tBOOST_FOREACH(file_output_location & out, outputs) {\n"
        "\t\t\t\t\t\tfile_output * output = out.first;\n"
        "\t\t\t\t\t\toutput->seek(out.second + output_size);\n"
        "\t\t\t\t\t\tbool success = output->write(buffer, size_t(n));"
    )
    new_copy_buf = (
        "\t\t\tboost::uint64_t output_size = 0;\n"
        "\t\t\tstatic const size_t kSerialBufSize = 1024u * 1024u;\n"
        "\t\t\tif(serial_copy_buf.size() < kSerialBufSize) {\n"
        "\t\t\t\tserial_copy_buf.resize(kSerialBufSize);\n"
        "\t\t\t}\n"
        "\t\t\twhile(!file_source->eof()) {\n"
        "\t\t\t\tstd::streamsize buffer_size = std::streamsize(serial_copy_buf.size());\n"
        "\t\t\t\tstd::streamsize n = file_source->read(serial_copy_buf.data(), buffer_size).gcount();\n"
        "\t\t\t\tif(n > 0) {\n"
        "\t\t\t\t\tBOOST_FOREACH(file_output_location & out, outputs) {\n"
        "\t\t\t\t\t\tfile_output * output = out.first;\n"
        "\t\t\t\t\t\toutput->seek(out.second + output_size);\n"
        "\t\t\t\t\t\tbool success = output->write(serial_copy_buf.data(), size_t(n));"
    )
    assert old_copy_buf in ec, "serial copy buffer block not found in extract.cpp"
    ec = ec.replace(old_copy_buf, new_copy_buf, 1)

    old_chunk_loop = "\tBOOST_FOREACH(const Chunks::value_type & chunk, chunks) {"
    parallel_chunk_loop = """\tstd::vector<char> serial_copy_buf;
\ttrinity_fast::parallel_zlib_pool zlib_pool;
\tstd::vector<trinity_fast::zlib_job> zlib_jobs(zlib_pool.workers());
\tstd::vector<Chunks::const_iterator> zlib_batch_its(zlib_pool.workers());
\t
\tfor(Chunks::const_iterator chunk_it = chunks.begin(); chunk_it != chunks.end(); ) {
\t\tif(o.extract && !o.test && !o.list && slice_reader.get()) {
\t\t\tsize_t batch_n = 0;
\t\t\tChunks::const_iterator scan = chunk_it;
\t\t\twhile(scan != chunks.end() && batch_n < zlib_pool.workers()) {
\t\t\t\tconst stream::chunk & ck = scan->first;
\t\t\t\tif(ck.encryption != stream::Plaintext || scan->second.size() != 1) {
\t\t\t\t\tbreak;
\t\t\t\t}
\t\t\t\tconst Files::value_type & loc = *scan->second.begin();
\t\t\t\tconst stream::file & fl = loc.first;
\t\t\t\tconst setup::data_entry & de = info.data_entries[loc.second];
\t\t\t\tbool is_chunk_zlib = (ck.compression == stream::Zlib && fl.filter == stream::NoFilter);
\t\t\t\tbool is_file_zlib = (ck.compression == stream::Stored && fl.filter == stream::ZlibFilter);
\t\t\t\tif((!is_chunk_zlib && !is_file_zlib) || fl.offset != 0) {
\t\t\t\t\tbreak;
\t\t\t\t}
\t\t\t\tboost::uint64_t comp_sz = is_chunk_zlib ? ck.size : fl.size;
\t\t\t\tif(comp_sz == 0 || comp_sz > trinity_fast::kMaxParallelComp
\t\t\t\t   || de.uncompressed_size == 0 || de.uncompressed_size > trinity_fast::kMaxParallelUncomp) {
\t\t\t\t\tbreak;
\t\t\t\t}
\t\t\t\tzlib_batch_its[batch_n] = scan;
\t\t\t\tzlib_jobs[batch_n].in_size = size_t(comp_sz);
\t\t\t\tzlib_jobs[batch_n].expected_out = size_t(de.uncompressed_size);
\t\t\t\tbatch_n++;
\t\t\t\t++scan;
\t\t\t}
\t\t\tif(batch_n >= 2) {
\t\t\t\tstatic const char kChunkMagic[4] = { 'z', 'l', 'b', 0x1a };
\t\t\t\tfor(size_t bi = 0; bi < batch_n; bi++) {
\t\t\t\t\tconst stream::chunk & ck = zlib_batch_its[bi]->first;
\t\t\t\t\tif(!slice_reader->seek(ck.first_slice, ck.offset)) {
\t\t\t\t\t\tthrow stream::chunk_error("could not seek to chunk start");
\t\t\t\t\t}
\t\t\t\t\tchar magic[4];
\t\t\t\t\tif(slice_reader->read(magic, 4) != 4 || std::memcmp(magic, kChunkMagic, 4) != 0) {
\t\t\t\t\t\tthrow stream::chunk_error("bad chunk magic");
\t\t\t\t\t}
\t\t\t\t\ttrinity_fast::zlib_job & job = zlib_jobs[bi];
\t\t\t\t\tif(job.in_buf.size() < job.in_size) {
\t\t\t\t\t\tjob.in_buf.resize(job.in_size);
\t\t\t\t\t}
\t\t\t\t\tstd::streamsize got = slice_reader->read(job.in_buf.data(), std::streamsize(job.in_size));
\t\t\t\t\tif(got != std::streamsize(job.in_size)) {
\t\t\t\t\t\tthrow stream::chunk_error("zlib error: truncated slice read");
\t\t\t\t\t}
\t\t\t\t}
\t\t\t\tzlib_pool.run_batch(zlib_jobs, batch_n);
\t\t\t\tfor(size_t bi = 0; bi < batch_n; bi++) {
\t\t\t\t\tconst trinity_fast::zlib_job & job = zlib_jobs[bi];
\t\t\t\t\tif(!job.error.empty()) {
\t\t\t\t\t\tthrow std::runtime_error(job.error);
\t\t\t\t\t}
\t\t\t\t\tconst Files::value_type & loc = *zlib_batch_its[bi]->second.begin();
\t\t\t\t\tconst std::vector<output_location> & output_locations = files_for_location[loc.second];
\t\t\t\t\tconst setup::data_entry & data = info.data_entries[loc.second];
\t\t\t\t\tboost::ptr_vector<file_output> single_outputs;
\t\t\t\t\ttypedef std::pair<file_output *, boost::uint64_t> file_output_location;
\t\t\t\t\tstd::vector<file_output_location> outputs;
\t\t\t\t\tBOOST_FOREACH(const output_location & output_loc, output_locations) {
\t\t\t\t\t\tconst processed_file * fileinfo = output_loc.first;
\t\t\t\t\t\tfile_output * output = NULL;
\t\t\t\t\t\tif(fileinfo->is_multipart()) {
\t\t\t\t\t\t\tmulti_part_outputs::iterator mit = multi_outputs.find(fileinfo);
\t\t\t\t\t\t\tif(mit != multi_outputs.end()) {
\t\t\t\t\t\t\t\toutput = mit->second;
\t\t\t\t\t\t\t}
\t\t\t\t\t\t}
\t\t\t\t\t\tif(!output) {
\t\t\t\t\t\t\toutput = new file_output(o.output_dir, fileinfo, true, false);
\t\t\t\t\t\t\tif(fileinfo->is_multipart()) {
\t\t\t\t\t\t\t\tmulti_outputs.insert(fileinfo, output);
\t\t\t\t\t\t\t} else {
\t\t\t\t\t\t\t\tsingle_outputs.push_back(output);
\t\t\t\t\t\t\t}
\t\t\t\t\t\t}
\t\t\t\t\t\toutputs.push_back(file_output_location(output, output_loc.second));
\t\t\t\t\t}
\t\t\t\t\tBOOST_FOREACH(file_output_location & out, outputs) {
\t\t\t\t\t\tfile_output * output = out.first;
\t\t\t\t\t\toutput->seek(out.second);
\t\t\t\t\t\tif(!output->write(job.out_buf.data(), job.actual_out)) {
\t\t\t\t\t\t\tthrow std::runtime_error("Error writing file \\"" + output->path().string() + '"');
\t\t\t\t\t\t}
\t\t\t\t\t}
\t\t\t\t\textract_progress.update(boost::uint64_t(job.actual_out));
\t\t\t\t\tutil::time filetime = data.timestamp;
\t\t\t\t\tif(o.preserve_file_times && o.local_timestamps && !(data.options & data.TimeStampInUTC)) {
\t\t\t\t\t\tfiletime = util::to_local_time(filetime);
\t\t\t\t\t}
\t\t\t\t\tBOOST_FOREACH(file_output_location & out, outputs) {
\t\t\t\t\t\tfile_output * output = out.first;
\t\t\t\t\t\tif(!output || (output->file()->is_multipart() && !output->is_complete())) {
\t\t\t\t\t\t\tcontinue;
\t\t\t\t\t\t}
\t\t\t\t\t\toutput->close();
\t\t\t\t\t\tif(o.preserve_file_times && !util::set_file_time(output->path(), filetime, data.timestamp_nsec)) {
\t\t\t\t\t\t\tlog_warning << "Error setting timestamp on file " << output->path();
\t\t\t\t\t\t}
\t\t\t\t\t\tBOOST_FOREACH(file_output_location & other, outputs) {
\t\t\t\t\t\t\tif(other.first == output) {
\t\t\t\t\t\t\t\tother.first = NULL;
\t\t\t\t\t\t\t}
\t\t\t\t\t\t}
\t\t\t\t\t\tif(output->file()->is_multipart()) {
\t\t\t\t\t\t\tmulti_outputs.erase(output->file());
\t\t\t\t\t\t}
\t\t\t\t\t}
\t\t\t\t}
\t\t\t\tchunk_it = scan;
\t\t\t\tcontinue;
\t\t\t}
\t\t}
\t\tconst Chunks::value_type & chunk = *chunk_it++;"""
    assert old_chunk_loop in ec, "BOOST_FOREACH chunks loop not found in extract.cpp"
    ec = ec.replace(old_chunk_loop, parallel_chunk_loop, 1)

    old_close_block = (
        "\t\t\t\t// Adjust file timestamps\n"
        "\t\t\t\tif(o.extract && o.preserve_file_times) {\n"
        "\t\t\t\t\toutput->close();\n"
        "\t\t\t\t\tif(!util::set_file_time(output->path(), filetime, data.timestamp_nsec)) {\n"
        '\t\t\t\t\t\tlog_warning << "Error setting timestamp on file " << output->path();\n'
        "\t\t\t\t\t}\n"
        "\t\t\t\t}"
    )
    new_close_block = (
        "\t\t\t\t// Explicitly close and verify every extracted file before touching timestamps\n"
        "\t\t\t\tif(o.extract) {\n"
        "\t\t\t\t\toutput->close();\n"
        "\t\t\t\t\tif(o.preserve_file_times && !util::set_file_time(output->path(), filetime, data.timestamp_nsec)) {\n"
        '\t\t\t\t\t\tlog_warning << "Error setting timestamp on file " << output->path();\n'
        "\t\t\t\t\t}\n"
        "\t\t\t\t}"
    )
    assert old_close_block in ec, "preserve_file_times close block not found in extract.cpp"
    ec = ec.replace(old_close_block, new_close_block, 1)
    extract_cpp.write_text(ec, encoding="utf-8", newline="\n")


def patch_slice(slice_hpp: pathlib.Path, slice_cpp: pathlib.Path) -> None:
    sh = slice_hpp.read_text(encoding="utf-8")
    old_sh_inc = '#include "util/fstream.hpp"'
    new_sh_inc = '#include "util/fstream.hpp"\n#include "util/trinity_fast_io.hpp"'
    assert old_sh_inc in sh, "fstream.hpp include not found in slice.hpp"
    sh = sh.replace(old_sh_inc, new_sh_inc, 1)

    old_sh_fields = (
        "\t// Streams\n"
        "\tutil::ifstream ifs; //!< File input stream used when reading from external slices.\n"
        "\tstd::istream * is;  //!< Input stream to read from."
    )
    new_sh_fields = (
        "\t// Streams\n"
        "\tutil::ifstream ifs; //!< Fallback stream (unused for external slices on Win32).\n"
        "#ifdef _WIN32\n"
        "\ttrinity_fast::buffered_win_reader win_reader; //!< 4 MiB buffered Win32 reader for external slices.\n"
        "#endif\n"
        "\tstd::istream * is;  //!< Input stream to read from (NULL when win_reader is active)."
    )
    assert old_sh_fields in sh, "slice_reader stream fields not found in slice.hpp"
    sh = sh.replace(old_sh_fields, new_sh_fields, 1)

    old_sh_open = "\tbool is_open() { return (is != &ifs || ifs.is_open()); }"
    new_sh_open = (
        "#ifdef _WIN32\n"
        "\tbool is_open() { return (is != NULL || win_reader.is_open()); }\n"
        "#else\n"
        "\tbool is_open() { return (is != &ifs || ifs.is_open()); }\n"
        "#endif"
    )
    assert old_sh_open in sh, "slice_reader::is_open not found in slice.hpp"
    sh = sh.replace(old_sh_open, new_sh_open, 1)
    slice_hpp.write_text(sh, encoding="utf-8", newline="\n")

    sc = slice_cpp.read_text(encoding="utf-8")
    old_sc_ctor = (
        "slice_reader::slice_reader(const path_type & dirname, const std::string & basename,\n"
        "                           const std::string & basename2, size_t disk_slice_count)\n"
        "\t: data_offset(0),\n"
        "\t  dir(dirname), base_file(basename), base_file2(basename2),\n"
        "\t  slices_per_disk(disk_slice_count), current_slice(0), slice_size(0),\n"
        "\t  is(&ifs) { }"
    )
    new_sc_ctor = (
        "slice_reader::slice_reader(const path_type & dirname, const std::string & basename,\n"
        "                           const std::string & basename2, size_t disk_slice_count)\n"
        "\t: data_offset(0),\n"
        "\t  dir(dirname), base_file(basename), base_file2(basename2),\n"
        "\t  slices_per_disk(disk_slice_count), current_slice(0), slice_size(0),\n"
        "#ifdef _WIN32\n"
        "\t  is(NULL) { }\n"
        "#else\n"
        "\t  is(&ifs) { }\n"
        "#endif"
    )
    assert old_sc_ctor in sc, "slice_reader external ctor not found in slice.cpp"
    sc = sc.replace(old_sc_ctor, new_sc_ctor, 1)

    old_sc_open_file = """bool slice_reader::open_file(const path_type & file) {
\t
\tif(!boost::filesystem::exists(file)) {
\t\treturn false;
\t}
\t
\tlog_info << "Opening \\"" << color::cyan << file.string() << color::reset << '"';
\t
\tifs.close();
\tifs.clear();
\t
\tifs.open(file, std::ios_base::in | std::ios_base::binary | std::ios_base::ate);
\tif(ifs.fail()) {
\t\treturn false;
\t}
\t
\tstd::streampos file_size = ifs.tellg();
\tifs.seekg(0);
\t
\tchar magic[8];
\tif(ifs.read(magic, 8).fail()) {
\t\tifs.close();
\t\tthrow slice_error("could not read slice magic number in \\"" + file.string() + "\\"");
\t}
\tbool found = false;
\tfor(size_t i = 0; boost::size(slice_ids); i++) {
\t\tif(!std::memcmp(magic, slice_ids[i], 8)) {
\t\t\tfound = true;
\t\t\tbreak;
\t\t}
\t}
\tif(!found) {
\t\tifs.close();
\t\tthrow slice_error("bad slice magic number in \\"" + file.string() + "\\"");
\t}
\t
\tslice_size = util::load<boost::uint32_t>(ifs);
\tif(ifs.fail()) {
\t\tifs.close();
\t\tthrow slice_error("could not read slice size in \\"" + file.string() + "\\"");
\t} else if(std::streampos(slice_size) > file_size) {
\t\tifs.close();
\t\tstd::ostringstream oss;
\t\toss << "bad slice size in " << file << ": " << slice_size << " > " << file_size;
\t\tthrow slice_error(oss.str());
\t} else if(std::streampos(slice_size) < ifs.tellg()) {
\t\tifs.close();
\t\tstd::ostringstream oss;
\t\toss << "bad slice size in " << file << ": " << slice_size << " < " << ifs.tellg();
\t\tthrow slice_error(oss.str());
\t}
\t
\treturn true;
}"""
    new_sc_open_file = """bool slice_reader::open_file(const path_type & file) {
\t
\tif(!boost::filesystem::exists(file)) {
\t\treturn false;
\t}
\t
\tlog_info << "Opening \\"" << color::cyan << file.string() << color::reset << '"';
\t
#ifdef _WIN32
\tis = NULL;
\tif(!win_reader.open(file.wstring())) {
\t\treturn false;
\t}
\tboost::uint64_t file_size = win_reader.size();
\tchar header[12];
\tif(win_reader.read(header, 12) != 12) {
\t\twin_reader.close();
\t\tthrow slice_error("could not read slice header in \\"" + file.string() + "\\"");
\t}
\tbool found = false;
\tfor(size_t i = 0; i < boost::size(slice_ids); i++) {
\t\tif(!std::memcmp(header, slice_ids[i], 8)) {
\t\t\tfound = true;
\t\t\tbreak;
\t\t}
\t}
\tif(!found) {
\t\twin_reader.close();
\t\tthrow slice_error("bad slice magic number in \\"" + file.string() + "\\"");
\t}
\tslice_size = boost::uint32_t(boost::uint8_t(header[8]))
\t           | (boost::uint32_t(boost::uint8_t(header[9])) << 8)
\t           | (boost::uint32_t(boost::uint8_t(header[10])) << 16)
\t           | (boost::uint32_t(boost::uint8_t(header[11])) << 24);
\tif(boost::uint64_t(slice_size) > file_size || slice_size < 12u) {
\t\twin_reader.close();
\t\tstd::ostringstream oss;
\t\toss << "bad slice size in " << file << ": " << slice_size << " (file size " << file_size << ")";
\t\tthrow slice_error(oss.str());
\t}
\treturn true;
#else
\tifs.close();
\tifs.clear();
\t
\tifs.open(file, std::ios_base::in | std::ios_base::binary | std::ios_base::ate);
\tif(ifs.fail()) {
\t\treturn false;
\t}
\t
\tstd::streampos file_size = ifs.tellg();
\tifs.seekg(0);
\t
\tchar magic[8];
\tif(ifs.read(magic, 8).fail()) {
\t\tifs.close();
\t\tthrow slice_error("could not read slice magic number in \\"" + file.string() + "\\"");
\t}
\tbool found = false;
\tfor(size_t i = 0; i < boost::size(slice_ids); i++) {
\t\tif(!std::memcmp(magic, slice_ids[i], 8)) {
\t\t\tfound = true;
\t\t\tbreak;
\t\t}
\t}
\tif(!found) {
\t\tifs.close();
\t\tthrow slice_error("bad slice magic number in \\"" + file.string() + "\\"");
\t}
\t
\tslice_size = util::load<boost::uint32_t>(ifs);
\tif(ifs.fail()) {
\t\tifs.close();
\t\tthrow slice_error("could not read slice size in \\"" + file.string() + "\\"");
\t} else if(std::streampos(slice_size) > file_size) {
\t\tifs.close();
\t\tstd::ostringstream oss;
\t\toss << "bad slice size in " << file << ": " << slice_size << " > " << file_size;
\t\tthrow slice_error(oss.str());
\t} else if(std::streampos(slice_size) < ifs.tellg()) {
\t\tifs.close();
\t\tstd::ostringstream oss;
\t\toss << "bad slice size in " << file << ": " << slice_size << " < " << ifs.tellg();
\t\tthrow slice_error(oss.str());
\t}
\t
\treturn true;
#endif
}"""
    assert old_sc_open_file in sc, "slice_reader::open_file not found in slice.cpp"
    sc = sc.replace(old_sc_open_file, new_sc_open_file, 1)

    old_sc_open_close = "\tcurrent_slice = slice;\n\tis = &ifs;\n\tifs.close();"
    new_sc_open_close = (
        "\tcurrent_slice = slice;\n"
        "#ifdef _WIN32\n"
        "\tis = NULL;\n"
        "\twin_reader.close();\n"
        "#else\n"
        "\tis = &ifs;\n"
        "\tifs.close();\n"
        "#endif"
    )
    assert old_sc_open_close in sc, "slice_reader::open close lines not found in slice.cpp"
    sc = sc.replace(old_sc_open_close, new_sc_open_close, 1)

    old_sc_seek_read = """bool slice_reader::seek(size_t slice, boost::uint32_t offset) {
\t
\tseek(slice);
\t
\toffset += data_offset;
\t
\tif(offset > slice_size) {
\t\treturn false;
\t}
\t
\tif(is->seekg(offset).fail()) {
\t\treturn false;
\t}
\t
\treturn true;
}

std::streamsize slice_reader::read(char * buffer, std::streamsize bytes) {
\t
\tseek(current_slice);
\t
\tstd::streamsize nread = 0;
\t
\twhile(bytes > 0) {
\t\t
\t\tboost::uint32_t read_pos = boost::uint32_t(is->tellg());
\t\tif(read_pos > slice_size) {
\t\t\tbreak;
\t\t}
\t\tboost::uint32_t remaining = slice_size - read_pos;
\t\tif(!remaining) {
\t\t\tseek(current_slice + 1);
\t\t\tread_pos = boost::uint32_t(is->tellg());
\t\t\tif(read_pos > slice_size) {
\t\t\t\tbreak;
\t\t\t}
\t\t\tremaining = slice_size - read_pos;
\t\t}
\t\t
\t\tboost::uint64_t toread = std::min(boost::uint64_t(remaining), boost::uint64_t(bytes));
\t\ttoread = std::min(toread, boost::uint64_t(std::numeric_limits<std::streamsize>::max()));
\t\tif(is->read(buffer, std::streamsize(toread)).fail()) {
\t\t\tbreak;
\t\t}
\t\t
\t\tstd::streamsize read = is->gcount();
\t\tnread += read, buffer += read, bytes -= read;
\t}
\t
\treturn (nread != 0 || bytes == 0) ? nread : -1;
}"""
    new_sc_seek_read = """bool slice_reader::seek(size_t slice, boost::uint32_t offset) {
\t
\tseek(slice);
\t
\toffset += data_offset;
\t
\tif(offset > slice_size) {
\t\treturn false;
\t}
\t
#ifdef _WIN32
\tif(!is) {
\t\treturn win_reader.seek(boost::uint64_t(offset));
\t}
#endif
\tif(is->seekg(offset).fail()) {
\t\treturn false;
\t}
\t
\treturn true;
}

std::streamsize slice_reader::read(char * buffer, std::streamsize bytes) {
\t
\tseek(current_slice);
\t
\tstd::streamsize nread = 0;
\t
\twhile(bytes > 0) {
\t\t
#ifdef _WIN32
\t\tboost::uint32_t read_pos = !is ? boost::uint32_t(win_reader.tell()) : boost::uint32_t(is->tellg());
#else
\t\tboost::uint32_t read_pos = boost::uint32_t(is->tellg());
#endif
\t\tif(read_pos > slice_size) {
\t\t\tbreak;
\t\t}
\t\tboost::uint32_t remaining = slice_size - read_pos;
\t\tif(!remaining) {
\t\t\tseek(current_slice + 1);
#ifdef _WIN32
\t\t\tread_pos = !is ? boost::uint32_t(win_reader.tell()) : boost::uint32_t(is->tellg());
#else
\t\t\tread_pos = boost::uint32_t(is->tellg());
#endif
\t\t\tif(read_pos > slice_size) {
\t\t\t\tbreak;
\t\t\t}
\t\t\tremaining = slice_size - read_pos;
\t\t}
\t\t
\t\tboost::uint64_t toread = std::min(boost::uint64_t(remaining), boost::uint64_t(bytes));
\t\ttoread = std::min(toread, boost::uint64_t(std::numeric_limits<std::streamsize>::max()));
#ifdef _WIN32
\t\tif(!is) {
\t\t\tsize_t got = win_reader.read(buffer, size_t(toread));
\t\t\tif(got == 0) {
\t\t\t\tbreak;
\t\t\t}
\t\t\tnread += std::streamsize(got), buffer += got, bytes -= std::streamsize(got);
\t\t\tcontinue;
\t\t}
#endif
\t\tif(is->read(buffer, std::streamsize(toread)).fail()) {
\t\t\tbreak;
\t\t}
\t\t
\t\tstd::streamsize read = is->gcount();
\t\tnread += read, buffer += read, bytes -= read;
\t}
\t
\treturn (nread != 0 || bytes == 0) ? nread : -1;
}"""
    assert old_sc_seek_read in sc, "slice_reader::seek/read not found in slice.cpp"
    sc = sc.replace(old_sc_seek_read, new_sc_seek_read, 1)
    slice_cpp.write_text(sc, encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
