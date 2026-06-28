#include <stdio.h>
#include <stdint.h>
#include <time.h>

static uint64_t compute(void) __attribute__((noinline));

static uint64_t compute(void) {
    uint64_t x = 1;
    for (long i = 0; i < 100000000L; i++) {
        x = x * 6364136223846793005ULL + 1442695040888963407ULL;
    }
    return x;
}

int main(void) {
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    uint64_t x = compute();
    clock_gettime(CLOCK_MONOTONIC, &t1);

    long long micros = (long long)(t1.tv_sec - t0.tv_sec) * 1000000LL
                     + (long long)(t1.tv_nsec - t0.tv_nsec) / 1000LL;

    printf("RESULT %lld\n", (long long)x);
    printf("MICROS %lld\n", micros);
    return 0;
}
