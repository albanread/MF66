\ Sum of Collatz step-counts for n = 1..1,000,000
: clen ( n -- steps ) 0 swap
   begin dup 1 > while
     dup 2 mod 0= if 2/ else 3 * 1+ then  swap 1+ swap
   repeat drop ;
: collatz ( N -- sum ) 0 swap 1+ 1 do i clen + loop ;
utime drop  1000000 collatz  utime drop
rot -  ." RESULT " swap . cr  ." MICROS " . cr
