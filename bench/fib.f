\ fib(34) recursive, base fib(0)=fib(1)=1  =>  F(35) = 9227465
: fib ( n -- f ) dup 2 < if drop 1 else dup 1- recurse swap 2 - recurse + then ;
utime drop  34 fib  utime drop
rot -  ." RESULT " swap . cr  ." MICROS " . cr
