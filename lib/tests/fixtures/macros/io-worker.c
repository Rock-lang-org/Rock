#define _DEFAULT_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <unistd.h>

static void emit(int fd, const char *bytes, size_t length) {
    while (length) {
        ssize_t count = write(fd, bytes, length);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) _exit(12);
        bytes += count;
        length -= (size_t)count;
    }
}

int main(int argc, char **argv) {
    (void)argc;
    struct rlimit core = {0, 0};
    setrlimit(RLIMIT_CORE, &core);
    alarm(4); /* Independent watchdog, including a regressed compiler transport. */
    char path[PATH_MAX];
    snprintf(path, sizeof(path), "%s.pid", argv[0]);
    FILE *pid = fopen(path, "w");
    if (!pid) return 10;
    fprintf(pid, "%d", (int)getpid());
    fclose(pid);
    const char *mode = strrchr(argv[0], '/');
    mode = mode ? mode + 1 : argv[0];
    if (!strcmp(mode, "blocked-stdin")) for (;;) pause();
    if (!strcmp(mode, "broken-pipe")) {
        close(0);
        emit(2, "stdin closed", 12);
        for (;;) pause();
    }
    char buffer[4096];
    ssize_t count;
    while ((count = read(0, buffer, !strcmp(mode, "partial") ? 257 : sizeof(buffer))) > 0) {
        if (!strcmp(mode, "partial")) usleep(500);
    }
    if (!strcmp(mode, "blocked-stdout")) {
        emit(1, "\7", 1);
        for (;;) pause();
    }
    if (!strcmp(mode, "oversized-stdout") || !strcmp(mode, "oversized-stderr")) {
        memset(buffer, 'x', sizeof(buffer));
        int fd = !strcmp(mode, "oversized-stdout") ? 1 : 2;
        for (int i = 0; i < 257; i++) emit(fd, buffer, sizeof(buffer));
        return 0;
    }
    if (!strcmp(mode, "malformed")) { emit(1, "invalid", 7); emit(2, "bad wire", 8); return 0; }
    if (!strcmp(mode, "crash")) { emit(2, "crash marker", 12); raise(SIGSEGV); return 11; }
    snprintf(path, sizeof(path), "%s.response", argv[0]);
    FILE *response = fopen(path, "rb");
    if (!response) return 13;
    while ((count = (ssize_t)fread(buffer, 1, !strcmp(mode, "partial") ? 1 : sizeof(buffer), response)) > 0) {
        emit(1, buffer, (size_t)count);
        if (!strcmp(mode, "partial")) usleep(500);
    }
    fclose(response);
    if (!strcmp(mode, "nonzero")) { emit(2, "nonzero marker", 14); return 7; }
    if (!strcmp(mode, "blocked-shutdown")) for (;;) pause();
    return 0;
}
