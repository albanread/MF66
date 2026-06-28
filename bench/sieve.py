import time

N = 1000000


def sieve():
    flags = bytearray(N)  # init: all prime (0) -- inside the timed region

    i = 2
    while i * i < N:
        if flags[i] == 0:
            j = i * i
            while j < N:
                flags[j] = 1
                j += i
        i += 1

    count = 0
    for i in range(2, N):
        if flags[i] == 0:
            count += 1
    return count


t0 = time.perf_counter()
result = sieve()
t1 = time.perf_counter()

micros = round((t1 - t0) * 1e6)
print("RESULT %d" % result)
print("MICROS %d" % micros)
