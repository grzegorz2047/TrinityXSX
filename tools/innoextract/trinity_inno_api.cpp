// SPDX-License-Identifier: Zlib
// In-process C API wrapper around dscharrer/innoextract for TrinityXSX (Windows + PS5 Wine64).
// Allows extracting DRM-free GOG offline Inno Setup installers (including multi-part .bin slices)
// inside xcl-library.exe without fork/exec (required under PS5 WINELOADERNOEXEC=1).

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <exception>
#include <string>

#include <boost/filesystem/operations.hpp>
#include <boost/filesystem/path.hpp>

#include "cli/extract.hpp"
#include "util/console.hpp"
#include "util/log.hpp"

namespace fs = boost::filesystem;

typedef void (__cdecl *TrinityInnoProgressFn)(uint64_t done_bytes, uint64_t total_bytes, void *ctx);

static std::atomic<TrinityInnoProgressFn> g_progress_fn{nullptr};
static std::atomic<void *> g_progress_ctx{nullptr};

extern "C" void trinity_inno_init_utf8_locale();

// Called from our tiny hook in src/util/console.cpp (progress::update)
extern "C" void trinity_inno_notify_progress(uint64_t done_bytes, uint64_t total_bytes) {
    TrinityInnoProgressFn fn = g_progress_fn.load(std::memory_order_acquire);
    if (fn) {
        fn(done_bytes, total_bytes, g_progress_ctx.load(std::memory_order_acquire));
    }
}

static std::wstring utf8_to_wide(const char *s) {
    if (!s || !*s) return std::wstring();
    int len = MultiByteToWideChar(CP_UTF8, 0, s, -1, nullptr, 0);
    if (len <= 1) return std::wstring();
    std::wstring out((size_t)(len - 1), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s, -1, &out[0], len);
    return out;
}

static void write_err(char *buf, size_t cap, const char *msg) {
    if (!buf || cap == 0) return;
    if (!msg) msg = "unknown error";
    std::snprintf(buf, cap, "%s", msg);
}

extern "C" __declspec(dllexport) int __cdecl trinity_inno_extract_utf8(
    const char *setup_exe_utf8,
    const char *output_dir_utf8,
    TrinityInnoProgressFn progress_cb,
    void *progress_ctx,
    char *err_buf,
    size_t err_buf_len)
{
    if (!setup_exe_utf8 || !output_dir_utf8 || !*setup_exe_utf8 || !*output_dir_utf8) {
        write_err(err_buf, err_buf_len, "invalid installer or output path");
        return 1;
    }

    g_progress_ctx.store(progress_ctx, std::memory_order_release);
    g_progress_fn.store(progress_cb, std::memory_order_release);

    int rc = 0;
    try {
        trinity_inno_init_utf8_locale();
        std::wstring in_w = utf8_to_wide(setup_exe_utf8);
        std::wstring out_w = utf8_to_wide(output_dir_utf8);
        fs::path installer(in_w);
        fs::path out_dir(out_w);

        fs::create_directories(out_dir);

        logger::quiet = true;
        progress::set_enabled(progress_cb != nullptr);

        extract_options opts;
        opts.quiet = true;
        opts.silent = true;
        opts.warn_unused = false;
        opts.extract = true;
        opts.gog = true;
        opts.gog_galaxy = true;
        opts.extract_unknown = true;
        opts.extract_temp = false;
        opts.preserve_file_times = false;
        opts.local_timestamps = false;
        opts.codepage = 0;
        opts.filenames.set_expand(true);
        opts.filenames.set_lowercase(false);
        opts.collisions = OverwriteCollisions;
        opts.output_dir = out_dir;

        process_file(installer, opts);
    } catch (const std::exception &e) {
        write_err(err_buf, err_buf_len, e.what());
        rc = 2;
    } catch (...) {
        write_err(err_buf, err_buf_len, "unexpected C++ exception during Inno extraction");
        rc = 3;
    }

    g_progress_fn.store(nullptr, std::memory_order_release);
    g_progress_ctx.store(nullptr, std::memory_order_release);
    return rc;
}
