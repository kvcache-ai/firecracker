// Copyright 2026 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

/* Own three mlocked ordinary pages. Only explicit commands change them. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

static void fail(const char *operation) {
    perror(operation);
    exit(1);
}

static void status(int value) {
    FILE *out = fopen("/tmp/dirty-pages.status", "w");
    if (!out) fail("status");
    if (fprintf(out, "%d\n", value) < 0 || fclose(out)) fail("status write");
}

int main(void) {
    long page_size = sysconf(_SC_PAGESIZE);
    if (page_size <= 0) fail("page size");
    size_t length = 3 * (size_t)page_size;
    unsigned char *pages = mmap(NULL, length, PROT_READ | PROT_WRITE,
                               MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (pages == MAP_FAILED) fail("mmap");
    if (madvise(pages, length, MADV_NOHUGEPAGE) || mlock(pages, length)) fail("mlock");
    for (int i = 0; i < 3; i++) memset(pages + i * page_size, 0x11 * (i + 1), page_size);

    status(0);
    int previous = 0;
    for (;;) {
        FILE *command = fopen("/tmp/dirty-pages.command", "r");
        int value = previous;
        if (command) {
            if (fscanf(command, "%d", &value) != 1) value = previous;
            fclose(command);
        }
        if (value != previous) {
            if (value == 1) memset(pages, 0xa1, page_size);
            else if (value == 2) {
                memset(pages + page_size, 0xb2, page_size);
                memset(pages + 2 * page_size, 0, page_size);
            } else if (value == 4) {
                for (size_t i = 0; i < length; i++) {
                    unsigned char expected = i < (size_t)page_size ? 0xa1 :
                        (i < 2 * (size_t)page_size ? 0xb2 : 0);
                    if (pages[i] != expected) {
                        status(-1);
                        return 1;
                    }
                }
            } else return 2;
            previous = value;
            status(value);
        }
        usleep(10000);
    }
}
