\ fib.f — iterative Fibonacci
: fib  ( n -- fib ) 0 1 rot 0 ?do  over + swap  loop drop ;
." fib 10 = "  10 fib . cr
." fib 20 = "  20 fib . cr
