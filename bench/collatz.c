/* collatz benchmark — sum of Collatz step-counts for n = 1..1000000
 * Matches the reference Forth: signed 64-bit math, loop-based.
 * Expected RESULT 131434424. */
#include <stdio.h>
#include <time.h>

/* Non-inlined so -O2 cannot fold the whole computation to a constant. */
__attribute__((noinline))
static long long collatz_sum(long long N) {
    long long sum = 0;
    for (long long i = 1; i <= N; i++) {
        long long n = i;
        long long count = 0;
        while (n > 1) {
            if ((n % 2) == 0) {
                n = n / 2;
            } else {
                n = 3 * n + 1;
            }
            count++;
        }
        sum += count;
    }
    return sum;
}

int main(void) {
    const long long N = 1000000;

    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    long long result = collatz_sum(N);
    clock_gettime(CLOCK_MONOTONIC, &t1);

    long long micros = (long long)(t1.tv_sec - t0.tv_sec) * 1000000LL
                     + (long long)(t1.tv_nsec - t0.tv_nsec) / 1000LL;

    printf("RESULT %lld\n", result);
    printf("MICROS %lld\n", micros);
    return 0;
}
