#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

static int mode;
static int calls;

void mock_set_mode(int64_t next) {
    mode = (int)next;
    calls = 0;
}

int64_t mock_calls(void) {
    return calls;
}

#if defined(TEST_CLOSE)
int close(int fd) {
    (void)fd;
    calls++;
    if (mode == 0) return 0;
    errno = mode == 1 ? EINTR : EIO;
    return -1;
}

int socket(int domain, int type, int protocol) {
    (void)domain; (void)type; (void)protocol;
    return 200;
}

int bind(int fd, const struct sockaddr *addr, socklen_t length) {
    (void)fd; (void)addr; (void)length;
    if (mode != 3) return 0;
    errno = EADDRINUSE;
    return -1;
}

int listen(int fd, int backlog) {
    (void)fd; (void)backlog;
    if (mode != 4) return 0;
    errno = ENOBUFS;
    return -1;
}

int connect(int fd, const struct sockaddr *addr, socklen_t length) {
    (void)fd; (void)addr; (void)length;
    if (mode != 5) return 0;
    errno = ECONNREFUSED;
    return -1;
}
#elif defined(TEST_SLEEP)
int nanosleep(const struct timespec *request, struct timespec *remaining) {
    calls++;
    if (mode == 2 && request->tv_sec == INT64_C(9223372036854775)
                  && request->tv_nsec == 807000000) return 0;
    if (mode == 0) {
        if (calls == 1 && request->tv_sec == 2 && request->tv_nsec == 500000000) {
            *remaining = (struct timespec){1, 250000000};
            errno = EINTR;
            return -1;
        }
        if (calls == 2 && request->tv_sec == 1 && request->tv_nsec == 250000000) {
            *remaining = (struct timespec){0, 125000000};
            errno = EINTR;
            return -1;
        }
        if (calls == 3 && request->tv_sec == 0 && request->tv_nsec == 125000000) return 0;
    }
    errno = EINVAL;
    return -1;
}
#elif defined(TEST_YIELD)
int sched_yield(void) {
    errno = EINVAL;
    return -1;
}
#elif defined(TEST_DETACH)
int pthread_detach(pthread_t thread) {
    (void)thread;
    // pthread functions return an error number; they do not use errno.
    errno = 0;
    return EINVAL;
}
#endif
