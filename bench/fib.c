#include <stdio.h>
#include <time.h>

/* Recursive Fibonacci mirroring the Forth:
   : fib ( n -- f ) dup 2 < if drop 1 else dup 1- recurse swap 2 - recurse + then ;
   base case fib(0)=fib(1)=1, fib(n)=fib(n-1)+fib(n-2) */
static long long fib(long long n) {
    if (n < 2) return 1;
    return fib(n - 1) + fib(n - 2);
}

int main(void) {
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    long long result = fib(34);
    clock_gettime(CLOCK_MONOTONIC, &t1);

    long long micros = (long long)(t1.tv_sec - t0.tv_sec) * 1000000LL
                     + (long long)(t1.tv_nsec - t0.tv_nsec) / 1000LL;

    printf("RESULT %lld\n", result);
    printf("MICROS %lld\n", micros);
    return 0;
}
