/* SPDX-License-Identifier: MIT */
/* Minimal C startup entry point for the native PS5 TrinityXSX launcher. */
#include <stddef.h>
#include <stdint.h>

typedef void (*crt_fn_t)(void);

extern void _init_env(void *process_params);
extern int atexit(crt_fn_t callback);
extern void exit(int status) __attribute__((noreturn));
extern int main(int argc, char **argv);

extern crt_fn_t __preinit_array_start[] __attribute__((weak));
extern crt_fn_t __preinit_array_end[] __attribute__((weak));
extern crt_fn_t __init_array_start[] __attribute__((weak));
extern crt_fn_t __init_array_end[] __attribute__((weak));
extern crt_fn_t __fini_array_start[] __attribute__((weak));
extern crt_fn_t __fini_array_end[] __attribute__((weak));

static void run_init_list(crt_fn_t *first, crt_fn_t *last)
{
    if (!first || !last) return;
    while (first != last) {
        (*first++)();
    }
}

static void run_fini_list(void)
{
    crt_fn_t *first = __fini_array_start;
    crt_fn_t *last = __fini_array_end;
    if (!first || !last) return;
    while (last != first) {
        (*--last)();
    }
}

__attribute__((noreturn, visibility("default")))
void _start(void *process_params, crt_fn_t loader_teardown)
{
    int argc = *(const int *)process_params;
    char **argv = (char **)((uint8_t *)process_params + sizeof(uint64_t));

    _init_env(process_params);
    if (loader_teardown) {
        (void)atexit(loader_teardown);
    }
    (void)atexit(run_fini_list);
    run_init_list(__preinit_array_start, __preinit_array_end);
    run_init_list(__init_array_start, __init_array_end);
    exit(main(argc, argv));
}
