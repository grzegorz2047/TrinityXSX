/* SPDX-License-Identifier: MIT */
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#ifdef TRINITYXSX_WITH_SEED
#include <errno.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>
#endif

/* The native SDK supplies this system service. It returns on failure. */
int sceSystemServiceLoadExec(const char *path, char *const arguments[]);
#ifdef TRINITYXSX_WITH_SEED
#define STORAGE_PARENT "/data/homebrew"
#define STORAGE_ROOT STORAGE_PARENT "/PPSA83601"
#define SEED_ROOT "/app0/data-seed"
#define PROFILE_INDEX "/profiles/profiles.lst"
#define LIBRARY_EXE "/prefix/drive_c/XCL/xcl-library.exe"
#define RESTART_FLAG "/restart.flag"

int trinityxsx_prepare_storage(const char *source, const char *parent, const char *root, const char **step);
int trinityxsx_sync_runtime(const char *source, const char *root, const char **step);

static int library_ready(const char *index)
{
    int fd = open(index, O_RDONLY | O_NOFOLLOW);
    if (fd < 0) return 0;
    struct stat info;
    int result = fstat(fd, &info);
    int error = errno;
    close(fd);
    if (result == 0 && S_ISREG(info.st_mode)) return 1;
    errno = result == 0 ? EINVAL : error;
    return 0;
}

static void boot_log(const char *stage, int status)
{
    char line[128];
    int length = snprintf(line, sizeof(line), "TrinityXSX %s status=%d\n", stage, status);
    if (length <= 0 || (size_t)length >= sizeof(line)) return;
    int fd = open(STORAGE_ROOT "/startup.log", O_WRONLY | O_CREAT | O_APPEND | O_NOFOLLOW, 0644);
    if (fd >= 0) {
        (void)write(fd, line, (size_t)length);
        close(fd);
        return;
    }
    fputs(line, stderr);
}
#endif

/* Keep the target pointers in the PS5 linker's read-only relocation section. */
#if defined(__clang__) && defined(__ELF__)
__attribute__((used))
#endif
static const char *const targets[] = { "/app0/runtime/pw_runner.bin", "exit" };

int main(int argc, char **argv)
{
    char profile[] = "profile=trinityxsx-library";
    char cycle[] = "cycle=1";
    char *arguments[] = { profile, cycle, NULL };
    const char *target = targets[0];
    char *const *target_arguments = arguments;

    for (int i = 0; i < argc; ++i) {
        if (argv[i] && strcmp(argv[i], "launcher=1") == 0) {
#ifdef TRINITYXSX_WITH_SEED
            if (unlink(STORAGE_ROOT RESTART_FLAG) == 0) {
                /* TrinityXSX requested a clean restart (e.g. to apply Frame Generation). */
                boot_log("restart-requested", 0);
                break;
            }
#endif
            /* Wine asked to return after an exit or startup failure. */
            target = targets[1];
            target_arguments = NULL;
            break;
        }
    }

#ifdef TRINITYXSX_WITH_SEED
    if (target_arguments) {
        boot_log("storage-start", (int)getuid());
        if (!library_ready(STORAGE_ROOT PROFILE_INDEX) || !library_ready(STORAGE_ROOT LIBRARY_EXE)) {
            const char *failed_step = "storage-seed";
            if (trinityxsx_prepare_storage(SEED_ROOT, STORAGE_PARENT, STORAGE_ROOT, &failed_step) != 0) {
                boot_log(failed_step, errno);
                (void)sceSystemServiceLoadExec(targets[1], NULL);
                return 1;
            }
            if (!library_ready(STORAGE_ROOT PROFILE_INDEX) || !library_ready(STORAGE_ROOT LIBRARY_EXE)) {
                boot_log("library-incomplete", errno);
                (void)sceSystemServiceLoadExec(targets[1], NULL);
                return 1;
            }
        } else {
            const char *failed_step = "runtime-sync";
            int synced = trinityxsx_sync_runtime(SEED_ROOT, STORAGE_ROOT, &failed_step);
            if (synced < 0) {
                /* Under PS5 UID 1 sandbox, overwriting UID 0 files returns EPERM/EACCES;
                 * pw_runner.bin repeats the sync immediately after Lapy elevates to UID 0. */
                if (errno == EPERM || errno == EACCES) {
                    boot_log("runtime-sync-deferred", 0);
                } else {
                    boot_log(failed_step, errno);
                }
            } else if (synced > 0) {
                boot_log("runtime-updated", 0);
            }
        }
        boot_log("runner-start", 0);
    }
#endif

    int result = sceSystemServiceLoadExec(target, target_arguments);
#ifdef TRINITYXSX_WITH_SEED
    boot_log("load-exec-returned", result);
#endif
    fprintf(stderr, "TrinityXSX: could not load %s (0x%08x)\n", target, (unsigned int)result);
    return 1;
}
