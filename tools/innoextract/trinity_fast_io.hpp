// SPDX-License-Identifier: Zlib
// High-throughput Win32 I/O buffers & parallel zlib worker pool for trinity_innoextract.dll.
// Cuts Wine syscall overhead (~1000x fewer read/write/seek syscalls) and parallelizes
// independent GOG Galaxy / Inno Setup zlib chunks across CPU cores on PS5 Zen 2 + M.2 SSD.
#ifndef TRINITY_FAST_IO_HPP
#define TRINITY_FAST_IO_HPP

#include <algorithm>
#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

#include <zlib.h>

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif

namespace trinity_fast {

static const size_t kSliceReadBufBytes  = 4u * 1024u * 1024u; // 4 MiB sequential read-ahead
static const size_t kFileWriteBufBytes  = 4u * 1024u * 1024u; // 4 MiB sequential write buffer
static const size_t kMaxParallelComp    = 24u * 1024u * 1024u;
static const size_t kMaxParallelUncomp  = 32u * 1024u * 1024u;

#ifdef _WIN32
inline std::wstring to_long_win_path(std::wstring wpath) {
	if(wpath.size() >= 248 && wpath.compare(0, 4, L"\\\\?\\") != 0 && wpath.size() >= 2 && wpath[1] == L':') {
		std::replace(wpath.begin(), wpath.end(), L'/', L'\\');
		wpath = L"\\\\?\\" + wpath;
	}
	return wpath;
}

// 4 MiB buffered read-only Win32 file reader with zero-syscall tell() and in-buffer seek().
class buffered_win_reader {
	HANDLE handle_;
	uint64_t file_size_;
	uint64_t pos_;        // logical read position
	uint64_t hw_pos_;     // actual Win32 file pointer
	uint64_t buf_start_;  // file offset of buf_[0]
	size_t buf_len_;      // valid bytes in buf_
	std::vector<char> buf_;

	bool seek_hw(uint64_t off) {
		if(hw_pos_ == off) {
			return true;
		}
		LARGE_INTEGER li;
		li.QuadPart = LONGLONG(off);
		if(!SetFilePointerEx(handle_, li, NULL, FILE_BEGIN)) {
			return false;
		}
		hw_pos_ = off;
		return true;
	}

	bool refill() {
		buf_len_ = 0;
		buf_start_ = pos_;
		if(pos_ >= file_size_) {
			return false;
		}
		if(!seek_hw(pos_)) {
			return false;
		}
		DWORD want = DWORD(std::min<uint64_t>(buf_.size(), file_size_ - pos_));
		DWORD got = 0;
		if(!ReadFile(handle_, buf_.data(), want, &got, NULL) || got == 0) {
			return false;
		}
		hw_pos_ += got;
		buf_len_ = size_t(got);
		return true;
	}

public:
	buffered_win_reader()
		: handle_(INVALID_HANDLE_VALUE)
		, file_size_(0)
		, pos_(0)
		, hw_pos_(0)
		, buf_start_(0)
		, buf_len_(0)
	{ }

	~buffered_win_reader() {
		close();
	}

	void close() {
		if(handle_ != INVALID_HANDLE_VALUE) {
			CloseHandle(handle_);
			handle_ = INVALID_HANDLE_VALUE;
		}
		file_size_ = 0;
		pos_ = 0;
		hw_pos_ = 0;
		buf_start_ = 0;
		buf_len_ = 0;
	}

	bool is_open() const {
		return handle_ != INVALID_HANDLE_VALUE;
	}

	uint64_t size() const {
		return file_size_;
	}

	uint64_t tell() const {
		return pos_;
	}

	bool open(const std::wstring & wpath_raw) {
		close();
		std::wstring wpath = to_long_win_path(wpath_raw);
		HANDLE h = CreateFileW(
			wpath.c_str(),
			GENERIC_READ,
			FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
			NULL,
			OPEN_EXISTING,
			FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN,
			NULL);
		if(h == INVALID_HANDLE_VALUE) {
			return false;
		}
		LARGE_INTEGER sz;
		if(!GetFileSizeEx(h, &sz) || sz.QuadPart < 0) {
			CloseHandle(h);
			return false;
		}
		handle_ = h;
		file_size_ = uint64_t(sz.QuadPart);
		pos_ = 0;
		hw_pos_ = 0;
		buf_start_ = 0;
		buf_len_ = 0;
		if(buf_.empty()) {
			buf_.resize(kSliceReadBufBytes);
		}
		return true;
	}

	bool seek(uint64_t offset) {
		if(handle_ == INVALID_HANDLE_VALUE || offset > file_size_) {
			return false;
		}
		pos_ = offset;
		return true;
	}

	size_t read(char * dst, size_t n) {
		if(handle_ == INVALID_HANDLE_VALUE || n == 0) {
			return 0;
		}
		size_t total = 0;
		while(total < n && pos_ < file_size_) {
			if(pos_ >= buf_start_ && pos_ < buf_start_ + buf_len_) {
				size_t off = size_t(pos_ - buf_start_);
				size_t avail = buf_len_ - off;
				size_t take = std::min(avail, n - total);
				std::memcpy(dst + total, buf_.data() + off, take);
				pos_ += take;
				total += take;
				continue;
			}
			// Large direct read straight into caller buffer when >= half buffer size
			size_t rem = n - total;
			if(rem >= (kSliceReadBufBytes / 2)) {
				if(!seek_hw(pos_)) {
					break;
				}
				DWORD want = DWORD(std::min<uint64_t>(rem, std::min<uint64_t>(0x40000000u, file_size_ - pos_)));
				DWORD got = 0;
				if(!ReadFile(handle_, dst + total, want, &got, NULL) || got == 0) {
					break;
				}
				hw_pos_ += got;
				pos_ += got;
				total += size_t(got);
				continue;
			}
			if(!refill()) {
				break;
			}
		}
		return total;
	}
};
#endif // _WIN32

struct zlib_job {
	std::vector<char> in_buf;
	std::vector<char> out_buf;
	size_t in_size;
	size_t expected_out;
	size_t actual_out;
	std::string error;

	zlib_job() : in_size(0), expected_out(0), actual_out(0) { }
};

inline bool inflate_zlib_job(zlib_job & job) {
	job.error.clear();
	job.actual_out = 0;
	if(job.expected_out > 0 && job.out_buf.size() < job.expected_out) {
		job.out_buf.resize(job.expected_out);
	} else if(job.out_buf.empty()) {
		job.out_buf.resize(65536u);
	}

	z_stream zs;
	std::memset(&zs, 0, sizeof(zs));
	int init_rc = inflateInit(&zs);
	if(init_rc != Z_OK) {
		job.error = "zlib error: inflateInit failed";
		return false;
	}

	zs.next_in = reinterpret_cast<Bytef *>(job.in_buf.data());
	zs.avail_in = static_cast<uInt>(job.in_size);

	size_t out_pos = 0;
	int rc = Z_OK;
	for(;;) {
		if(out_pos == job.out_buf.size()) {
			size_t next_sz = job.out_buf.size() + std::max<size_t>(job.out_buf.size() / 2, 262144u);
			if(next_sz > kMaxParallelUncomp * 2) {
				inflateEnd(&zs);
				job.error = "zlib error: decompressed chunk exceeded safety limit";
				return false;
			}
			job.out_buf.resize(next_sz);
		}
		zs.next_out = reinterpret_cast<Bytef *>(job.out_buf.data() + out_pos);
		zs.avail_out = static_cast<uInt>(job.out_buf.size() - out_pos);
		rc = inflate(&zs, Z_NO_FLUSH);
		out_pos = job.out_buf.size() - size_t(zs.avail_out);
		if(rc == Z_STREAM_END) {
			break;
		}
		if(rc != Z_OK) {
			const char * msg = zs.msg ? zs.msg : "iostream error";
			inflateEnd(&zs);
			job.error = std::string("zlib error: ") + msg;
			return false;
		}
		if(zs.avail_in == 0 && zs.avail_out > 0) {
			inflateEnd(&zs);
			job.error = "zlib error: truncated compressed stream";
			return false;
		}
	}

	inflateEnd(&zs);
	job.actual_out = out_pos;
	return true;
}

// Persistent worker pool for batched parallel zlib decompression.
class parallel_zlib_pool {
	std::vector<std::thread> threads_;
	std::mutex mu_;
	std::condition_variable cv_work_;
	std::condition_variable cv_done_;
	bool stop_;
	uint64_t generation_;
	std::vector<zlib_job> * active_jobs_;
	size_t active_count_;
	std::atomic<size_t> next_idx_;
	size_t completed_;

	void worker_loop() {
		uint64_t seen_gen = 0;
		for(;;) {
			std::vector<zlib_job> * jobs = NULL;
			size_t count = 0;
			{
				std::unique_lock<std::mutex> lk(mu_);
				cv_work_.wait(lk, [&]() { return stop_ || generation_ != seen_gen; });
				if(stop_) {
					return;
				}
				seen_gen = generation_;
				jobs = active_jobs_;
				count = active_count_;
			}
			for(;;) {
				size_t idx = next_idx_.fetch_add(1, std::memory_order_relaxed);
				if(idx >= count) {
					break;
				}
				inflate_zlib_job((*jobs)[idx]);
			}
			{
				std::lock_guard<std::mutex> lk(mu_);
				completed_++;
				if(completed_ == threads_.size()) {
					cv_done_.notify_one();
				}
			}
		}
	}

public:
	explicit parallel_zlib_pool(size_t num_workers = 0)
		: stop_(false)
		, generation_(0)
		, active_jobs_(NULL)
		, active_count_(0)
		, next_idx_(0)
		, completed_(0)
	{
		if(num_workers == 0) {
			unsigned hc = std::thread::hardware_concurrency();
			if(hc < 2) {
				num_workers = 2;
			} else if(hc > 6) {
				num_workers = 6;
			} else {
				num_workers = size_t(hc);
			}
		}
		threads_.reserve(num_workers);
		for(size_t i = 0; i < num_workers; i++) {
			threads_.emplace_back(&parallel_zlib_pool::worker_loop, this);
		}
	}

	~parallel_zlib_pool() {
		{
			std::lock_guard<std::mutex> lk(mu_);
			stop_ = true;
		}
		cv_work_.notify_all();
		for(size_t i = 0; i < threads_.size(); i++) {
			if(threads_[i].joinable()) {
				threads_[i].join();
			}
		}
	}

	size_t workers() const {
		return threads_.size();
	}

	void run_batch(std::vector<zlib_job> & jobs, size_t count) {
		if(count == 0) {
			return;
		}
		if(count == 1) {
			inflate_zlib_job(jobs[0]);
			return;
		}
		{
			std::lock_guard<std::mutex> lk(mu_);
			active_jobs_ = &jobs;
			active_count_ = count;
			next_idx_.store(0, std::memory_order_relaxed);
			completed_ = 0;
			generation_++;
		}
		cv_work_.notify_all();
		{
			std::unique_lock<std::mutex> lk(mu_);
			cv_done_.wait(lk, [&]() { return completed_ == threads_.size(); });
			active_jobs_ = NULL;
			active_count_ = 0;
		}
	}
};

} // namespace trinity_fast

#endif // TRINITY_FAST_IO_HPP
