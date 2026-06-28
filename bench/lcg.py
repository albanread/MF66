import time

MASK = 0xFFFFFFFFFFFFFFFF


def compute():
    x = 1
    for _ in range(100000000):
        x = (x * 6364136223846793005 + 1442695040888963407) & MASK
    return x


t0 = time.perf_counter()
x = compute()
t1 = time.perf_counter()

if x >= 2 ** 63:
    x -= 2 ** 64

micros = round((t1 - t0) * 1e6)
print("RESULT %d" % x)
print("MICROS %d" % micros)
