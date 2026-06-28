#!/usr/bin/env python3
# collatz benchmark — sum of Collatz step-counts for n = 1..1000000
# Matches the reference Forth: loop-based, same algorithm. Expected RESULT 131434424.
import time


def collatz_sum(N):
    total = 0
    for i in range(1, N + 1):
        n = i
        count = 0
        while n > 1:
            if n % 2 == 0:
                n = n // 2
            else:
                n = 3 * n + 1
            count += 1
        total += count
    return total


def main():
    N = 1000000
    t0 = time.perf_counter()
    result = collatz_sum(N)
    t1 = time.perf_counter()
    micros = round((t1 - t0) * 1e6)
    print("RESULT %d" % result)
    print("MICROS %d" % micros)


if __name__ == "__main__":
    main()
