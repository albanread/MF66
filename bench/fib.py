import sys
import time

# Recursive Fibonacci mirroring the Forth:
# : fib ( n -- f ) dup 2 < if drop 1 else dup 1- recurse swap 2 - recurse + then ;
# base case fib(0)=fib(1)=1, fib(n)=fib(n-1)+fib(n-2)


def fib(n):
    if n < 2:
        return 1
    return fib(n - 1) + fib(n - 2)


def main():
    sys.setrecursionlimit(10000)
    t0 = time.perf_counter()
    result = fib(34)
    t1 = time.perf_counter()
    micros = round((t1 - t0) * 1e6)
    print("RESULT %d" % result)
    print("MICROS %d" % micros)


if __name__ == "__main__":
    main()
