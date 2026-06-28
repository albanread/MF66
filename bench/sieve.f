\ Sieve of Eratosthenes: count primes below 1,000,000 (expect 78498). Includes array init.
create flags 1000000 allot
: sieve ( -- count )
   1000000 0 do 0 flags i + c! loop          \ init: all prime (0)
   2 begin dup dup * 1000000 < while
       dup flags + c@ 0= if
         dup dup *
         begin dup 1000000 < while
           1 over flags + c!  over +
         repeat drop
       then  1+
     repeat drop
   0  1000000 2 do flags i + c@ 0= if 1+ then loop ;
utime drop  sieve  utime drop
rot -  ." RESULT " swap . cr  ." MICROS " . cr
