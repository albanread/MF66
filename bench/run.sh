#!/usr/bin/env bash
# Reproduce the MF66 vs C vs Python benchmark table (best-of-N, serial).
# Each program prints "RESULT <x>" and "MICROS <compute-microseconds>" using its
# own monotonic, compute-only timer. Run serially — no concurrency — so there is
# no CPU contention to skew the numbers.
#
#   ./bench/run.sh [N]        # N = repetitions, default 5
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ENG="$ROOT/target/release/mf66"
[ -x "$ENG" ] || { echo "build the release engine first: cargo build --release --bin mf66"; exit 1; }
BIN="$(mktemp -d)"; trap 'rm -rf "$BIN"' EXIT
N="${1:-5}"

minmicros() { # run "$@" N times, echo "RESULT MIN_MICROS"
  local best="" res="" line m r i
  for i in $(seq 1 "$N"); do
    line=$("$@" < /dev/null 2>/dev/null)
    r=$(printf '%s\n' "$line" | awk '/RESULT/{print $2}')
    m=$(printf '%s\n' "$line" | awk '/MICROS/{print $2}')
    [ -n "$m" ] || continue
    res="$r"; { [ -z "$best" ] || [ "$m" -lt "$best" ]; } && best="$m"
  done
  echo "$res ${best:-NA}"
}

printf "%-9s | %-20s | %10s | %10s | %10s\n" bench result forth_us c_us py_us
printf -- "----------+----------------------+------------+------------+------------\n"
for b in fib collatz sieve lcg; do
  cc -O2 -o "$BIN/$b" "$ROOT/bench/$b.c" || { echo "$b: C compile failed"; continue; }
  read -r fr fu < <(minmicros "$ENG" "$ROOT/bench/$b.f")
  read -r cr cu < <(minmicros "$BIN/$b")
  read -r pr pu < <(minmicros python3 "$ROOT/bench/$b.py")
  printf "%-9s | %-20s | %10s | %10s | %10s\n" "$b" "$fr" "$fu" "$cu" "$pu"
  [ "$fr" = "$cr" ] && [ "$fr" = "$pr" ] || printf "   !! MISMATCH forth=%s c=%s py=%s\n" "$fr" "$cr" "$pr"
done
read -r tr tu < <(minmicros "$ENG" "$ROOT/bench/tlcg.f")
printf "%-9s | %-20s | %10s | %10s | %10s\n" "tlcg" "$tr" "$tu" "(=lcg)" "(=lcg)"
