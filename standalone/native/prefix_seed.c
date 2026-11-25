/* SPDX-License-Identifier: MIT */
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int directory_status(const char *path, struct stat *info)
{
    int fd = open(path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW);
    if (fd < 0) return errno == ENOENT ? 0 : -1;
    int result = info ? fstat(fd, info) : 0;
    int error = errno;
    close(fd);
    errno = error;
    return result == 0 ? 1 : -1;
}

static int joined(char *output, size_t size, const char *directory, const char *name)
{
    int length = snprintf(output, size, "%s/%s", directory, name);
    if (length >= 0 && (size_t)length < size) return 0;
    errno = ENAMETOOLONG;
    return -1;
}

static int copy_file(int input, const char *destination)
{
    int output = open(destination, O_WRONLY | O_CREAT | O_TRUNC | O_NOFOLLOW | O_NONBLOCK, 0644);
    if (output < 0) return -1;
    struct stat info;
    int failed = fstat(output, &info) != 0 || !S_ISREG(info.st_mode);

    char buffer[16384];
    while (!failed) {
        ssize_t count = read(input, buffer, sizeof(buffer));
        if (count == 0) break;
        if (count < 0) {
            if (errno == EINTR) continue;
            failed = 1;
            break;
        }
        for (ssize_t written = 0; written < count;) {
            ssize_t chunk = write(output, buffer + written, (size_t)(count - written));
            if (chunk < 0 && errno == EINTR) continue;
            if (chunk <= 0) {
                failed = 1;
                break;
            }
            written += chunk;
        }
    }
    int error = errno;
    if (close(output) != 0) return -1;
    errno = error;
    return failed ? -1 : 0;
}

static int copy_tree(const char *source, const char *destination)
{
    struct stat info;
    int input = open(source, O_RDONLY | O_NOFOLLOW | O_NONBLOCK);
    if (input < 0) return -1;
    int result = fstat(input, &info);
    if (result == 0 && S_ISREG(info.st_mode)) {
        result = copy_file(input, destination);
        int error = errno;
        close(input);
        errno = error;
        return result;
    }
    int error = errno;
    close(input);
    if (result != 0 || !S_ISDIR(info.st_mode)) {
        errno = result != 0 ? error : EINVAL;
        return -1;
    }
    if (mkdir(destination, 0755) != 0 && directory_status(destination, NULL) != 1) return -1;

    DIR *directory = opendir(source);
    if (!directory) return -1;
    result = 0;
    char from[PATH_MAX], to[PATH_MAX];
    for (;;) {
        errno = 0;
        struct dirent *entry = readdir(directory);
        if (!entry) {
            if (errno) result = -1;
            break;
        }
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
        if (joined(from, sizeof(from), source, entry->d_name) != 0 ||
            joined(to, sizeof(to), destination, entry->d_name) != 0 || copy_tree(from, to) != 0) {
            result = -1;
            break;
        }
    }
    error = errno;
    closedir(directory);
    errno = error;
    return result;
}

static int ensure_dir(const char *path)
{
    if (mkdir(path, 0755) == 0) return 0;
    if (errno == EEXIST && directory_status(path, NULL) == 1) return 0;
    return -1;
}

/* Populate root/prefix and root/profiles from source if missing.
 * PS5 titles run as UID 1 while FTP-uploaded files have UID 0, so we do not
 * reject existing directories by st_uid; wineserver patch 0997 skips the
 * single-user config_dir owner check on __PROSPERO__. */
int trinityxsx_prepare_storage(const char *source, const char *parent, const char *root, const char **step)
{
    char prefix[PATH_MAX], profiles[PATH_MAX], from[PATH_MAX];
    *step = "storage-parent-mkdir";
    if (ensure_dir(parent) != 0) return -1;
    *step = "storage-root-mkdir";
    if (ensure_dir(root) != 0) return -1;
    if (joined(prefix, sizeof(prefix), root, "prefix") != 0 ||
        joined(profiles, sizeof(profiles), root, "profiles") != 0) {
        *step = "storage-path";
        return -1;
    }
    *step = "storage-prefix-copy";
    if (directory_status(prefix, NULL) != 1) {
        if (joined(from, sizeof(from), source, "prefix") != 0 || copy_tree(from, prefix) != 0) return -1;
    }
    *step = "storage-profiles-copy";
    if (directory_status(profiles, NULL) != 1) {
        if (joined(from, sizeof(from), source, "profiles") != 0 || copy_tree(from, profiles) != 0) return -1;
    }
    return 0;
}
