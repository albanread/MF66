#include <stdio.h>
#include <time.h>

#define N 1000000

static unsigned char flags[N];

/* Keep the compute out-of-line so -O2 cannot fold it to a constant. */
__attribute__((noinline))
static long long sieve(void) {
    long long i, j, count;

    /* init: all prime (0) -- inside the timed region */
    for (i = 0; i < N; i++) flags[i] = 0;

    /* mark composites starting from i*i */
    for (i = 2; i * i < N; i++) {
        if (flags[i] == 0) {
            for (j = i * i; j < N; j += i) flags[j] = 1;
        }
    }

    /* count primes in [2, N) */
    count = 0;
    for (i = 2; i < N; i++) {
        if (flags[i] == 0) count++;
    }
    return count;
}

int main(void) {
    struct timespec t0, t1;
    long long result;
    long long micros;

    clock_gettime(CLOCK_MONOTONIC, &t0);
    result = sieve();
    clock_gettime(CLOCK_MONOTONIC, &t1);

    micros = (long long)(t1.tv_sec - t0.tv_sec) * 1000000LL
           + (long long)(t1.tv_nsec - t0.tv_nsec) / 1000LL;

    printf("RESULT %lld\n", result);
    printf("MICROS %lld\n", micros);
    return 0;
}
