\ Same LCG as lcg.f but expressed as TAIL recursion (showcases the O(1) tail-call:
\ 100,000,000-deep RECURSE runs in constant return-stack space).
: tlcg ( x n -- x' ) dup 0= if drop exit then
   swap 6364136223846793005 * 1442695040888963407 + swap 1- recurse ;
utime drop  1 100000000 tlcg  utime drop
rot -  ." RESULT " swap . cr  ." MICROS " . cr
