#include <stddef.h>
#include <stdint.h>
#include <unistd.h>

extern void *__libc_malloc(size_t);
extern void __libc_free(void *);
static size_t allocations;
static size_t releases;

struct Callable {
    int64_t (*code)(void *);
    void *environment;
};

int64_t observe_copy_environment(const struct Callable *callback) {
    if (callback == NULL || callback->code == NULL || callback->environment == NULL) return -1;
    return callback->code(callback->environment);
}

int64_t observe_array_environments(const struct Callable *callbacks) {
    if (callbacks[0].environment == callbacks[1].environment ||
        callbacks[0].environment == callbacks[2].environment ||
        callbacks[1].environment == callbacks[2].environment) return -2;
    return observe_copy_environment(&callbacks[0]) + observe_copy_environment(&callbacks[2]);
}

void *malloc(size_t size) {
    void *value = __libc_malloc(size);
    if (value != NULL) ++allocations;
    return value;
}

void free(void *value) {
    if (value != NULL) ++releases;
    __libc_free(value);
}

static void count(size_t value) {
    char bytes[3 * sizeof(size_t)];
    size_t length = 0;
    do {
        bytes[length++] = (char)('0' + value % 10);
        value /= 10;
    } while (value != 0);
    while (length != 0) {
        if (write(STDERR_FILENO, &bytes[--length], 1) != 1) return;
    }
}

__attribute__((destructor)) static void report(void) {
    const char prefix[] = "COPY_ENV allocations=";
    const char middle[] = " releases=";
    if (write(STDERR_FILENO, prefix, sizeof(prefix) - 1) != (ssize_t)(sizeof(prefix) - 1)) return;
    count(allocations);
    if (write(STDERR_FILENO, middle, sizeof(middle) - 1) != (ssize_t)(sizeof(middle) - 1)) return;
    count(releases);
    (void)write(STDERR_FILENO, "\n", 1);
}
